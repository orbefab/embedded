//! SPI1, the controller through which the firmware sends commands to the flash, and the flash
//! chip behind it.
//!
//! A write of `SPI_MEM_CMD_REG` runs its one command at once and clears its bits, and a write
//! that starts more than one is refused. A user command sends the command byte and the address
//! when `SPI_MEM_USR_ADDR` is set, and leaves the MISO bytes from `SPI_MEM_W0_REG` on. A user
//! command while `SPI_MEM_MISC_REG` has `CS0_DIS` set or `CS1_DIS` clear is refused. That register
//! resets to `0x2`, CS0 on and CS1 off, and the fixed commands do not look at it. No command
//! the chip runs takes MOSI bytes, and the dummy cycles have no effect. The fixed commands `RDSR`, `WREN`, `WRDI` and
//! `RDID` run as their flash commands do. The controller's other registers are plain words.
//!
//! The chip is the board's 16 MB Boya BY25Q128, JEDEC ID `68 40 18`, its quad enable bit set.
//! It reads, and reports its ID and status. A status write can lock the flash for good, so it is
//! refused before any status bit changes, with or without `WREN`. The chip has no program or
//! erase here, so those are refused as a write, which Wi-Fi, Bluetooth and saved settings make.
//! A read past the flash's end and every other command the model does not know are refused too.
//! A refusal ends the run.
//! IDF v5.5.5, `components/soc/esp32s3/include/soc/spi_mem_reg.h`,
//! `components/hal/esp32s3/include/hal/spimem_flash_ll.h` and `components/spi_flash`.

const CMD: u32 = 0x0;
const ADDR: u32 = 0x4;
const USER: u32 = 0x18;
const USER1: u32 = 0x1c;
const USER2: u32 = 0x20;
const MISO_DLEN: u32 = 0x28;
const RD_STATUS: u32 = 0x2c;
const MISC: u32 = 0x34;
const W0: u32 = 0x58;
const BUFFER_WORDS: usize = 16;
const SIZE: u32 = 0x400;

const FLASH_RDID: u32 = 1 << 28;
const FLASH_RDSR: u32 = 1 << 27;
const FLASH_WRSR: u32 = 1 << 26;
const FLASH_WREN: u32 = 1 << 30;
const FLASH_WRDI: u32 = 1 << 29;
/// The status write `FLASH_WRSR` sends.
const WRSR: u8 = 0x01;
/// The other fixed commands, each with the flash command it sends.
const FIXED: [(u32, u8); 8] = [
    (1 << 31, 0x03),
    (1 << 25, 0x02),
    (1 << 24, 0x20),
    (1 << 23, 0xd8),
    (1 << 22, 0xc7),
    (1 << 21, 0xb9),
    (1 << 20, 0xab),
    (1 << 19, 0xa3),
];
const USR: u32 = 1 << 18;
/// The `SPI_MEM_CMD_REG` bits that start a command, which clear when it is done. `SPI_MEM_USR`
/// and the fixed commands each start one. `SPI_MEM_FLASH_PE` marks a user command as a program
/// or an erase.
pub(crate) const STARTS: u32 = 0xffff_0000 | USR | 1 << 17;
const COMMANDS: u32 = 0xfffc_0000;

const USR_COMMAND: u32 = 1 << 31;
const USR_ADDR: u32 = 1 << 30;
const USR_MISO: u32 = 1 << 28;
const MISO_HIGHPART: u32 = 1 << 24;
/// `SPI_MEM_MISC_REG` bit 0. Set disables the flash's chip select.
const CS0_DIS: u32 = 1;
/// `SPI_MEM_MISC_REG` bit 1. Clear selects the second chip, which this flash is not.
const CS1_DIS: u32 = 1 << 1;

const JEDEC_ID: [u8; 3] = [0x68, 0x40, 0x18];
/// What `RDID` reads with no flash on the bus, whose MISO line idles high.
const NO_FLASH_ID: [u8; 3] = [0xff; 3];
const WEL: u8 = 1 << 1;
const QE: u8 = 1 << 1;

/// Why the chip refused a flash command, which ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashRefusal {
    /// A command the model does not know.
    Command(u8),
    /// A page program or an erase, which the model does not simulate.
    Write(u8),
    /// A status write: `SPI_MEM_FLASH_WRSR`, or a user `0x01`, `0x31` or `0x11`.
    StatusWrite(u8),
    /// A user command with no command phase, or a write that sets only `SPI_MEM_FLASH_PE`.
    NoCommandPhase,
    /// One write of `SPI_MEM_CMD_REG` that starts more than one command.
    StartBits,
    /// A read whose bytes reach past the flash's end. A chip with no flash refuses every read.
    PastEnd(u8),
    /// A command whose address phase is longer than 32 bits.
    AddressBits(u8),
    /// A write of SPI0's `SPI_MEM_CMD_REG` that starts a command.
    Spi0,
    /// A write that would take a flash pad, GPIO26 to GPIO32, from SPI0 and SPI1: another
    /// IO_MUX function, its output enable, or a matrix output route.
    Pads,
    /// A user command while `CS0_DIS` is set or `CS1_DIS` is clear.
    Cs(u8),
}

#[derive(Clone, Debug)]
pub(crate) struct Spi1 {
    /// The controller's registers, by word.
    regs: Vec<u32>,
    /// The chip's three status registers.
    status: [u8; 3],
}

impl Default for Spi1 {
    fn default() -> Spi1 {
        let mut regs = vec![0; (SIZE / 4) as usize];
        regs[(MISC / 4) as usize] = CS1_DIS;
        Spi1 {
            regs,
            status: [0, QE, 0],
        }
    }
}

impl Spi1 {
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        self.regs.get((offset / 4) as usize).copied()
    }

    fn reg(&self, offset: u32) -> u32 {
        self.regs[(offset / 4) as usize]
    }

    /// Writes a register, and runs the command a write of `SPI_MEM_CMD_REG` starts.
    pub(crate) fn write(
        &mut self,
        offset: u32,
        value: u32,
        flash: &[u8],
    ) -> Result<bool, FlashRefusal> {
        let Some(word) = self.regs.get_mut((offset / 4) as usize) else {
            return Ok(false);
        };
        *word = value;
        if offset == CMD && value & STARTS != 0 {
            self.regs[0] = value & !STARTS;
            self.run(value, flash)?;
        }
        Ok(true)
    }

    fn run(&mut self, cmd: u32, flash: &[u8]) -> Result<(), FlashRefusal> {
        match cmd & COMMANDS {
            0 => return Err(FlashRefusal::NoCommandPhase),
            USR => return self.user(flash),
            FLASH_RDSR => self.regs[(RD_STATUS / 4) as usize] = u32::from(self.status[0]),
            FLASH_WREN => self.status[0] |= WEL,
            FLASH_WRDI => self.status[0] &= !WEL,
            FLASH_WRSR => return Err(FlashRefusal::StatusWrite(WRSR)),
            FLASH_RDID => {
                let [a, b, c] = jedec_id(flash);
                self.regs[(W0 / 4) as usize] = u32::from_le_bytes([a, b, c, 0]);
            }
            one if one.is_power_of_two() => {
                let (_, command) = FIXED.iter().find(|&&(bit, _)| bit == one).expect("fixed");
                return Err(refusal(*command));
            }
            _ => return Err(FlashRefusal::StartBits),
        }
        Ok(())
    }

    /// Runs the user command the registers describe.
    fn user(&mut self, flash: &[u8]) -> Result<(), FlashRefusal> {
        let user = self.reg(USER);
        if user & USR_COMMAND == 0 {
            return Err(FlashRefusal::NoCommandPhase);
        }
        let command = self.reg(USER2) as u8;
        let misc = self.reg(MISC);
        if misc & CS0_DIS != 0 || misc & CS1_DIS == 0 {
            return Err(FlashRefusal::Cs(command));
        }
        let addr = if user & USR_ADDR != 0 {
            let bits = (self.reg(USER1) >> 26) + 1;
            if bits > 32 {
                return Err(FlashRefusal::AddressBits(command));
            }
            Some(self.reg(ADDR) & (u32::MAX >> (32 - bits)))
        } else {
            None
        };
        let bytes = |dlen: u32| ((dlen & 0x3ff) as usize + 1).div_ceil(8);
        let buffer: Vec<u8> = self.regs[(W0 / 4) as usize..][..BUFFER_WORDS]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        let miso_len = if user & USR_MISO != 0 {
            bytes(self.reg(MISO_DLEN))
        } else {
            0
        };
        let miso = self.transact(command, addr, miso_len, flash)?;
        if !miso.is_empty() {
            let start = if user & MISO_HIGHPART != 0 { 32 } else { 0 };
            let mut buffer = buffer;
            let end = (start + miso.len()).min(64);
            buffer[start..end].copy_from_slice(&miso[..end - start]);
            for (k, word) in buffer.chunks(4).enumerate() {
                self.regs[(W0 / 4) as usize + k] =
                    u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            }
        }
        Ok(())
    }

    /// The bytes the chip answers `command` with, `len` of them.
    fn transact(
        &mut self,
        command: u8,
        addr: Option<u32>,
        len: usize,
        flash: &[u8],
    ) -> Result<Vec<u8>, FlashRefusal> {
        let repeat = |byte: u8| vec![byte; len];
        Ok(match command {
            0x9f => jedec_id(flash)
                .iter()
                .copied()
                .chain(std::iter::repeat(0))
                .take(len)
                .collect(),
            0x05 => repeat(self.status[0]),
            0x35 => repeat(self.status[1]),
            0x15 => repeat(self.status[2]),
            0x06 => {
                self.status[0] |= WEL;
                Vec::new()
            }
            0x04 => {
                self.status[0] &= !WEL;
                Vec::new()
            }
            0x01 | 0x31 | 0x11 => return Err(FlashRefusal::StatusWrite(command)),
            0x03 | 0x0b | 0x3b | 0x6b | 0xbb | 0xeb => {
                let start = addr.unwrap_or(0) as usize;
                match start.checked_add(len).and_then(|end| flash.get(start..end)) {
                    Some(bytes) => bytes.to_vec(),
                    None => return Err(FlashRefusal::PastEnd(command)),
                }
            }
            0xab | 0xb9 | 0x66 | 0x99 | 0x7a | 0x75 | 0xff => repeat(0),
            other => return Err(refusal(other)),
        })
    }
}

/// Why the chip refuses `command`, one it does not run: a write when it programs a page or
/// erases, as the BY25Q128's page program `0x02`, quad page program `0x32`, sector erase `0x20`,
/// block erases `0x52` and `0xd8` and chip erases `0x60` and `0xc7` do.
fn refusal(command: u8) -> FlashRefusal {
    match command {
        0x02 | 0x32 | 0x20 | 0x52 | 0xd8 | 0x60 | 0xc7 => FlashRefusal::Write(command),
        other => FlashRefusal::Command(other),
    }
}

fn jedec_id(flash: &[u8]) -> [u8; 3] {
    if flash.is_empty() {
        NO_FLASH_ID
    } else {
        JEDEC_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spi1() -> Spi1 {
        let mut spi1 = Spi1::default();
        spi1.regs[(USER1 / 4) as usize] = 23 << 26;
        spi1
    }

    /// Sends `command` with `addr` on `flash`, and returns the `len` bytes it reads.
    fn send(
        spi1: &mut Spi1,
        flash: &[u8],
        command: u8,
        addr: Option<u32>,
        len: u32,
    ) -> Result<Vec<u8>, FlashRefusal> {
        let mut user = USR_COMMAND;
        if let Some(addr) = addr {
            user |= USR_ADDR;
            spi1.write(ADDR, addr, flash).unwrap();
        }
        if len > 0 {
            user |= USR_MISO;
            spi1.write(MISO_DLEN, 8 * len - 1, flash).unwrap();
        }
        spi1.write(USER, user, flash).unwrap();
        spi1.write(USER2, 7 << 28 | u32::from(command), flash)
            .unwrap();
        let sent = spi1.write(CMD, USR, flash);
        assert_eq!(spi1.read(CMD), Some(0), "the command is done at once");
        sent.map(|_| spi1.read(W0).unwrap().to_le_bytes()[..len as usize].to_vec())
    }

    fn user(spi1: &mut Spi1, command: u8, addr: Option<u32>, len: u32) -> Vec<u8> {
        let flash: Vec<u8> = (0..=255).collect();
        send(spi1, &flash, command, addr, len).unwrap()
    }

    #[test]
    fn the_chip_reports_its_id_status_and_reads_from_an_address() {
        let mut spi1 = spi1();
        assert_eq!(user(&mut spi1, 0x9f, None, 3), [0x68, 0x40, 0x18]);
        assert_eq!(user(&mut spi1, 0x35, None, 1), [QE]);
        assert_eq!(
            user(&mut spi1, 0x03, Some(0x10), 4),
            [0x10, 0x11, 0x12, 0x13]
        );
        assert_eq!(
            user(&mut spi1, 0x03, Some(0xfc), 4),
            [0xfc, 0xfd, 0xfe, 0xff],
            "the last bytes"
        );
    }

    #[test]
    fn a_chip_with_no_flash_reads_its_id_as_ff() {
        let mut spi1 = spi1();
        assert_eq!(send(&mut spi1, &[], 0x9f, None, 3), Ok(vec![0xff; 3]));
        spi1.write(CMD, FLASH_RDID, &[]).unwrap();
        assert_eq!(spi1.read(W0), Some(0x00ff_ffff));
    }

    #[test]
    fn a_status_write_is_refused_before_any_status_bit_changes() {
        let flash = [0xff; 16];
        for enable in [false, true] {
            for command in [0x01, 0x31, 0x11] {
                let mut spi1 = spi1();
                if enable {
                    user(&mut spi1, 0x06, None, 0);
                }
                spi1.write(W0, 0, &flash).unwrap();
                spi1.write(0x24, 15, &flash).unwrap();
                spi1.write(USER, USR_COMMAND | 1 << 27, &flash).unwrap();
                spi1.write(USER2, 7 << 28 | u32::from(command), &flash)
                    .unwrap();
                assert_eq!(
                    spi1.write(CMD, USR, &flash),
                    Err(FlashRefusal::StatusWrite(command)),
                    "{command:#04x}, WREN {enable}"
                );
                let wel = if enable { WEL } else { 0 };
                assert_eq!(spi1.status, [wel, QE, 0], "{command:#04x}, WREN {enable}");
            }
            let mut spi1 = spi1();
            if enable {
                user(&mut spi1, 0x06, None, 0);
            }
            assert_eq!(
                spi1.write(CMD, FLASH_WRSR, &flash),
                Err(FlashRefusal::StatusWrite(0x01)),
                "the fixed WRSR, WREN {enable}"
            );
            assert_eq!(spi1.status[1], QE);
        }
    }

    #[test]
    fn an_erase_or_a_program_ends_the_run_as_a_write() {
        for command in [0x02, 0x32, 0x20, 0x52, 0xd8, 0x60, 0xc7] {
            let mut spi1 = spi1();
            spi1.write(USER, USR_COMMAND | USR_ADDR, &[]).unwrap();
            spi1.write(USER2, 7 << 28 | command, &[]).unwrap();
            assert_eq!(
                spi1.write(CMD, USR | 1 << 17, &[0xff]),
                Err(FlashRefusal::Write(command as u8)),
                "{command:#x}"
            );
        }
        for (bit, command) in [(25, 0x02), (24, 0x20), (23, 0xd8), (22, 0xc7)] {
            assert_eq!(
                spi1().write(CMD, 1 << bit, &[0xff]),
                Err(FlashRefusal::Write(command)),
                "the fixed {command:#x}"
            );
        }
        assert_eq!(
            spi1().write(CMD, 1 << 21, &[0xff]),
            Err(FlashRefusal::Command(0xb9)),
            "the fixed deep power-down"
        );
    }

    #[test]
    fn a_write_that_starts_two_commands_runs_neither() {
        let mut spi1 = spi1();
        assert_eq!(
            spi1.write(CMD, FLASH_WREN | 1 << 24, &[0xff]),
            Err(FlashRefusal::StartBits),
            "WREN and a sector erase"
        );
        assert_eq!(spi1.status[0], 0, "no WREN");
        spi1.write(RD_STATUS, 0xa5, &[0xff]).unwrap();
        assert_eq!(
            spi1.write(CMD, FLASH_RDSR | 1 << 25, &[0xff]),
            Err(FlashRefusal::StartBits),
            "RDSR and a page program"
        );
        assert_eq!(spi1.read(RD_STATUS), Some(0xa5), "no RDSR");
        assert_eq!(
            spi1.write(CMD, USR | FLASH_RDID, &[0xff]),
            Err(FlashRefusal::StartBits),
            "a user command and RDID"
        );
        assert_eq!(spi1.read(CMD), Some(0), "the start bits clear");
    }

    #[test]
    fn a_read_past_the_flash_or_on_no_flash_is_refused() {
        let flash: Vec<u8> = (0..=255).collect();
        let mut spi1 = spi1();
        assert_eq!(
            send(&mut spi1, &flash, 0x03, Some(0xfe), 4),
            Err(FlashRefusal::PastEnd(0x03))
        );
        assert_eq!(
            send(&mut spi1, &[], 0x03, Some(0), 1),
            Err(FlashRefusal::PastEnd(0x03)),
            "no flash"
        );
        assert_eq!(
            send(&mut spi1, &flash, 0xeb, Some(0xff_ffff), 4),
            Err(FlashRefusal::PastEnd(0xeb))
        );
    }

    #[test]
    fn an_address_longer_than_32_bits_is_refused() {
        let flash: Vec<u8> = (0..=255).collect();
        let mut spi1 = spi1();
        spi1.regs[(USER1 / 4) as usize] = 32 << 26;
        assert_eq!(
            send(&mut spi1, &flash, 0x03, Some(0x10), 4),
            Err(FlashRefusal::AddressBits(0x03)),
            "33 bits"
        );
        spi1.regs[(USER1 / 4) as usize] = 31 << 26;
        assert_eq!(
            send(&mut spi1, &flash, 0x03, Some(0x10), 1),
            Ok(vec![0x10]),
            "32 bits"
        );
    }

    #[test]
    fn a_user_command_with_the_wrong_chip_select_is_refused() {
        let flash: Vec<u8> = (0..=255).collect();
        let mut reset = spi1();
        assert_eq!(reset.read(MISC), Some(CS1_DIS), "CS0 on, CS1 off");
        assert_eq!(
            send(&mut reset, &flash, 0x03, Some(0x10), 1),
            Ok(vec![0x10]),
            "the reset select"
        );

        let mut cs0 = spi1();
        cs0.write(MISC, CS0_DIS | CS1_DIS, &flash).unwrap();
        assert_eq!(
            send(&mut cs0, &flash, 0x03, Some(0x10), 1),
            Err(FlashRefusal::Cs(0x03)),
            "CS0 disabled"
        );
        assert_eq!(cs0.read(W0), Some(0), "the read does not run");

        let mut cs1 = spi1();
        cs1.write(MISC, 0, &flash).unwrap();
        assert_eq!(
            send(&mut cs1, &flash, 0x9f, None, 3),
            Err(FlashRefusal::Cs(0x9f)),
            "CS1 enabled"
        );

        let mut bare = spi1();
        bare.write(MISC, CS0_DIS, &flash).unwrap();
        bare.write(USER, 0, &flash).unwrap();
        bare.write(USER2, 7 << 28 | 0x03, &flash).unwrap();
        assert_eq!(
            bare.write(CMD, USR, &flash),
            Err(FlashRefusal::NoCommandPhase),
            "no command phase is refused first"
        );

        let mut fixed = spi1();
        fixed.write(MISC, 0, &flash).unwrap();
        fixed.write(CMD, FLASH_RDSR, &flash).unwrap();
        assert_eq!(
            fixed.read(RD_STATUS),
            Some(0),
            "a fixed command ignores the select"
        );
    }
}
