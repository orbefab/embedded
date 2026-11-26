//! GDMA, the DMA controller of five channel pairs, as far as SPI2 and SPI3 send from it: an OUT
//! channel whose `GDMA_PERI_OUT_SEL` picks SPI2 or SPI3 and whose link was started reads its
//! descriptor list from SRAM when that SPI host starts a transfer that takes its data from DMA.
//!
//! A descriptor is three words: `size` in bits 0 to 11, `length` in bits 12 to 23, `suc_eof` in
//! bit 30 and `owner` in bit 31, then the buffer's address, then the next descriptor's. The
//! channel sends `length` bytes of each buffer in turn, through the one with `suc_eof`.
//! `GDMA_OUTLINK_ADDR` holds the low 20 bits of the first descriptor's address, in the data-bus
//! SRAM window. The model reads the whole list when the transfer starts, and the CPU cannot
//! change a byte of it while SPI2 or SPI3 sends from it, as `Soc::in_flight` says.
//!
//! An IN channel's link is kept and runs nothing. It may name a descriptor in SRAM only for
//! SPI2 or SPI3, which refuse every transfer that would fill it. An IN link with no descriptor in
//! SRAM fills nothing, as the AES channel IDF starts at address 0 beside each SHA hash does. An IN
//! link that names a descriptor in SRAM for another peripheral ends the run, and so does an OUT
//! link whose `GDMA_PERI_OUT_SEL` names none of SPI2, SPI3 or the SHA accelerator, or a change of
//! a channel's peri_sel while its link is started. No GDMA interrupt rises. The registers come
//! from IDF's `soc/esp32s3/register/soc/gdma_reg.h`, with the reset values its field comments
//! give.

use std::ops::Range;

use super::sha;
use super::spi2::Setting;

const CHANNELS: usize = 5;
/// The words of one channel pair, `GDMA_IN_CONF0_CHn_REG` to `GDMA_OUT_PERI_SEL_CHn_REG`.
const WORDS: usize = 0xc0 / 4;

const IN_CONF0: u32 = 0x00;
const IN_CONF1: u32 = 0x04;
const IN_INT_RAW: u32 = 0x08;
const IN_INT_ST: u32 = 0x0c;
const IN_INT_ENA: u32 = 0x10;
const IN_INT_CLR: u32 = 0x14;
const IN_LINK: u32 = 0x20;
const IN_WEIGHT: u32 = 0x3c;
const IN_PRI: u32 = 0x44;
const IN_PERI_SEL: u32 = 0x48;
/// The OUT registers sit this far above their IN twins.
const OUT: u32 = 0x60;

const RST: u32 = 1 << 0;
/// `GDMA_OUT_AUTO_WRBACK`, which writes each descriptor back once its buffer is sent.
pub(crate) const AUTO_WRBACK: u32 = 1 << 2;
/// `GDMA_MEM_TRANS_EN`, which copies memory to memory.
pub(crate) const MEM_TRANS_EN: u32 = 1 << 4;
/// `GDMA_OUT_CHECK_OWNER`.
const CHECK_OWNER: u32 = 1 << 12;
/// `GDMA_OUTLINK_ADDR`, `GDMA_OUTLINK_STOP`, `GDMA_OUTLINK_START`, `GDMA_OUTLINK_RESTART` and
/// `GDMA_OUTLINK_PARK`.
const OUTLINK_ADDR: u32 = 0xf_ffff;
const OUTLINK_STOP: u32 = 1 << 20;
const OUTLINK_START: u32 = 1 << 21;
pub(crate) const OUTLINK_RESTART: u32 = 1 << 22;
const OUTLINK_PARK: u32 = 1 << 23;
/// The IN link's stop, start, restart and park bits, one above the OUT link's.
const INLINK_STROBES: u32 = 0x7 << 21;
const INLINK_STOP: u32 = 1 << 21;
const INLINK_START: u32 = 1 << 22;
pub(crate) const INLINK_RESTART: u32 = 1 << 23;
const INLINK_PARK: u32 = 1 << 24;

/// `GDMA_PERI_OUT_SEL`'s values for SPI2 and SPI3 (`SOC_GDMA_TRIG_PERIPH_SPI2`, `..._SPI3`).
pub(crate) const SPI2: u32 = 0;
pub(crate) const SPI3: u32 = 1;

/// The data-bus SRAM window, where a descriptor and its buffer lie.
pub(crate) const DMA_RAM: Range<u32> = 0x3fc8_8000..0x3fd0_0000;
/// The high bits `GDMA_OUTLINK_ADDR` leaves out.
const LINK_BASE: u32 = 0x3fc0_0000;

/// The descriptors the data-bus SRAM holds apart, so a list longer than this goes round a loop.
const MOST_DESCRIPTORS: u32 = (DMA_RAM.end - DMA_RAM.start) / 12;
const SUC_EOF: u32 = 1 << 30;
const OWNER: u32 = 1 << 31;

/// A channel pair's registers at reset, by offset over 4.
const RESET: [(u32, u32); 8] = [
    (IN_CONF1, 0xc),
    (IN_LINK, 0x0110_0000),
    (IN_WEIGHT, 0xf00),
    (IN_PERI_SEL, 0x3f),
    (OUT, 0x8),
    (OUT + IN_LINK, OUTLINK_PARK),
    (OUT + IN_WEIGHT, 0xf00),
    (OUT + IN_PERI_SEL, 0x3f),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Gdma {
    regs: [[u32; WORDS]; CHANNELS],
    /// The descriptor each OUT channel's link started at, until a transfer takes it, or a stop
    /// or a channel reset drops it.
    started: [Option<u32>; CHANNELS],
    /// Whether each IN channel's link is started at a descriptor in SRAM, until a stop or a
    /// channel reset.
    receiving: [bool; CHANNELS],
}

impl Gdma {
    pub(crate) fn new() -> Gdma {
        let mut pair = [0; WORDS];
        for (offset, value) in RESET {
            pair[(offset / 4) as usize] = value;
        }
        Gdma {
            regs: [pair; CHANNELS],
            started: [None; CHANNELS],
            receiving: [false; CHANNELS],
        }
    }

    /// The channel and the offset in its pair of a kept register at `offset`.
    fn kept(offset: u32) -> Option<(usize, u32)> {
        let channel = (offset / 0xc0) as usize;
        let at = offset % 0xc0;
        let local = at % OUT;
        let kept = matches!(
            local,
            IN_CONF0..=IN_INT_CLR | IN_LINK | IN_WEIGHT | IN_PRI | IN_PERI_SEL
        );
        (channel < CHANNELS && kept).then_some((channel, at))
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        let (channel, at) = Self::kept(offset)?;
        let regs = &self.regs[channel];
        let word = |at: u32| regs[(at / 4) as usize];
        Some(match at % OUT {
            IN_INT_ST => word(at - IN_INT_ST + IN_INT_RAW) & word(at - IN_INT_ST + IN_INT_ENA),
            IN_INT_CLR => 0,
            IN_LINK if at >= OUT => {
                let park = if self.started[channel].is_some() {
                    0
                } else {
                    OUTLINK_PARK
                };
                word(at) & OUTLINK_ADDR | park
            }
            _ => word(at),
        })
    }

    /// Writes the register at `offset`, and returns whether the model keeps it, or the setting
    /// that refuses the write, which then leaves every register as it was.
    pub(crate) fn write(&mut self, offset: u32, value: u32) -> Result<bool, &'static str> {
        let Some((channel, at)) = Self::kept(offset) else {
            return Ok(false);
        };
        let out = at >= OUT;
        let base = if out { OUT } else { 0 };
        let peri_sel = self.regs[channel][((base + IN_PERI_SEL) / 4) as usize] & 0x3f;
        let listed = DMA_RAM.contains(&(LINK_BASE | value & OUTLINK_ADDR));
        let (start, fed, started): (u32, &[u32], bool) = if out {
            let fed = &[SPI2, SPI3, sha::PERIPHERAL];
            (OUTLINK_START, fed, self.started[channel].is_some())
        } else {
            (INLINK_START, &[SPI2, SPI3], self.receiving[channel])
        };
        let refused = if at == base + IN_LINK
            && value & start != 0
            && (out || listed)
            && !fed.contains(&peri_sel)
        {
            Some("link_peripheral")
        } else if at == base + IN_PERI_SEL && started && value & 0x3f != peri_sel {
            Some("peri_sel_while_started")
        } else {
            None
        };
        if let Some(setting) = refused {
            return Err(setting);
        }
        let word = &mut self.regs[channel][(at / 4) as usize];
        match at % OUT {
            IN_CONF0 => {
                *word = value;
                if value & RST != 0 {
                    if out {
                        self.started[channel] = None;
                    } else {
                        self.receiving[channel] = false;
                    }
                }
            }
            IN_INT_RAW | IN_INT_ST => {}
            IN_INT_CLR => {
                self.regs[channel][((at - IN_INT_CLR + IN_INT_RAW) / 4) as usize] &= !value
            }
            IN_LINK if out => {
                *word = value & OUTLINK_ADDR;
                if value & OUTLINK_STOP != 0 {
                    self.started[channel] = None;
                }
                if value & OUTLINK_START != 0 {
                    self.started[channel] = Some(LINK_BASE | value & OUTLINK_ADDR);
                }
            }
            IN_LINK => {
                *word = (value & !(INLINK_STROBES | INLINK_PARK)) | INLINK_PARK;
                if value & INLINK_STOP != 0 {
                    self.receiving[channel] = false;
                }
                if value & INLINK_START != 0 {
                    self.receiving[channel] = listed;
                }
            }
            _ => *word = value,
        }
        Ok(true)
    }

    /// The `bytes` bytes the OUT channel that feeds `peripheral` sends, read from its descriptor
    /// list through `byte`, which gives the byte at a data-bus SRAM address and is called for
    /// every byte of a descriptor or a buffer the transfer reads. The channel's link
    /// is then spent. The setting that refuses the transfer when no channel, or more than one,
    /// is started for `peripheral`, when a descriptor or buffer lies outside SRAM or fails the
    /// owner check, when the list goes round a loop, or when it does not hold exactly `bytes`
    /// bytes.
    pub(crate) fn send(
        &mut self,
        peripheral: u32,
        bytes: usize,
        mut byte: impl FnMut(u32) -> u8,
    ) -> Result<Vec<u8>, Setting> {
        let mut feeding = (0..CHANNELS).filter(|&channel| {
            self.started[channel].is_some()
                && self.regs[channel][((OUT + IN_PERI_SEL) / 4) as usize] & 0x3f == peripheral
        });
        let (Some(channel), None) = (feeding.next(), feeding.next()) else {
            return Err(Setting::DmaLink);
        };
        let check_owner = self.regs[channel][((OUT + IN_CONF1) / 4) as usize] & CHECK_OWNER != 0;
        let mut at = self.started[channel].expect("started");
        let mut data = Vec::with_capacity(bytes);
        for _ in 0..=MOST_DESCRIPTORS {
            let inside = |addr: u32, len: u32| {
                DMA_RAM.contains(&addr)
                    && u64::from(addr) + u64::from(len) <= u64::from(DMA_RAM.end)
            };
            if !at.is_multiple_of(4) || !inside(at, 12) {
                return Err(Setting::DmaDescriptor);
            }
            let [head, buffer, next] = [at, at + 4, at + 8]
                .map(|word| u32::from_le_bytes([0, 1, 2, 3].map(|k| byte(word + k))));
            let length = (head >> 12) & 0xfff;
            if (check_owner && head & OWNER == 0) || (length > 0 && !inside(buffer, length)) {
                return Err(Setting::DmaDescriptor);
            }
            data.extend((buffer..buffer + length).map(&mut byte));
            if data.len() > bytes {
                return Err(Setting::DmaLength);
            }
            if head & SUC_EOF != 0 {
                if data.len() != bytes {
                    return Err(Setting::DmaLength);
                }
                self.started[channel] = None;
                return Ok(data);
            }
            at = next;
        }
        Err(Setting::DmaDescriptor)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const CH: u32 = 2 * 0xc0;

    /// A memory of words, 0 where none was put.
    #[derive(Default)]
    struct Memory(BTreeMap<u32, u32>);

    impl Memory {
        fn put(&mut self, addr: u32, bytes: &[u8]) {
            for (k, &byte) in bytes.iter().enumerate() {
                let at = addr + k as u32;
                let word = self.0.entry(at & !3).or_default();
                *word = (*word & !(0xff << (8 * (at & 3)))) | u32::from(byte) << (8 * (at & 3));
            }
        }

        /// A descriptor at `at` for `buffer`'s `length` bytes, then `next`.
        fn descriptor(&mut self, at: u32, length: u32, eof: bool, buffer: u32, next: u32) {
            let head = OWNER | if eof { SUC_EOF } else { 0 } | length << 12 | length;
            for (k, word) in [head, buffer, next].into_iter().enumerate() {
                self.0.insert(at + 4 * k as u32, word);
            }
        }

        fn byte(&self) -> impl Fn(u32) -> u8 + '_ {
            |addr| (self.0.get(&(addr & !3)).copied().unwrap_or(0) >> (8 * (addr & 3))) as u8
        }
    }

    fn start(gdma: &mut Gdma, at: u32, peripheral: u32) {
        assert_eq!(gdma.write(CH + OUT + IN_PERI_SEL, peripheral), Ok(true));
        assert_eq!(gdma.write(CH + OUT, RST), Ok(true));
        assert_eq!(gdma.write(CH + OUT, 0), Ok(true));
        assert_eq!(
            gdma.write(CH + OUT + IN_LINK, at & OUTLINK_ADDR | OUTLINK_START),
            Ok(true)
        );
    }

    #[test]
    fn a_started_channel_sends_each_buffer_through_the_eof_and_is_then_spent() {
        let mut memory = Memory::default();
        memory.put(0x3fca_0001, b"hello ");
        memory.put(0x3fcb_0000, b"world");
        memory.descriptor(0x3fce_a5d8, 6, false, 0x3fca_0001, 0x3fce_a600);
        memory.descriptor(0x3fce_a600, 5, true, 0x3fcb_0000, 0);
        let mut gdma = Gdma::new();
        start(&mut gdma, 0x3fce_a5d8, SPI2);
        assert_eq!(
            gdma.read(CH + OUT + IN_LINK),
            Some(0xea5d8),
            "running, not parked"
        );
        assert_eq!(
            gdma.send(SPI2, 11, memory.byte()).as_deref(),
            Ok(&b"hello world"[..])
        );
        assert_eq!(gdma.read(CH + OUT + IN_LINK), Some(0xea5d8 | OUTLINK_PARK));
        assert_eq!(gdma.send(SPI2, 11, memory.byte()), Err(Setting::DmaLink));
    }

    #[test]
    fn a_list_that_does_not_hold_the_transfer_lies_outside_sram_or_loops_refuses_it() {
        let mut memory = Memory::default();
        memory.descriptor(0x3fce_0000, 4, true, 0x3fca_0000, 0);
        memory.descriptor(0x3fce_0010, 4, true, 0x4037_8000, 0);
        memory.descriptor(0x3fce_0020, 0, false, 0x3fca_0000, 0x3fce_0020);
        memory.descriptor(0x3fce_0030, 4, false, 0x3fca_0000, 0x3fce_0030);
        memory.descriptor(0x3fc8_0000, 4, true, 0x3fca_0000, 0);
        let refused = |at: u32, bytes: usize| {
            let mut gdma = Gdma::new();
            start(&mut gdma, at, SPI2);
            gdma.send(SPI2, bytes, memory.byte())
        };
        assert_eq!(refused(0x3fce_0000, 4).map(|data| data.len()), Ok(4));
        assert_eq!(refused(0x3fce_0000, 5), Err(Setting::DmaLength));
        assert_eq!(refused(0x3fce_0000, 3), Err(Setting::DmaLength));
        assert_eq!(refused(0x3fce_0010, 4), Err(Setting::DmaDescriptor));
        assert_eq!(refused(0x3fc0_0000, 4), Err(Setting::DmaDescriptor));
        assert_eq!(refused(0x3fc8_0000, 4), Err(Setting::DmaDescriptor));
        assert_eq!(
            refused(0x3fce_0020, 4),
            Err(Setting::DmaDescriptor),
            "a loop"
        );
        assert_eq!(
            refused(0x3fce_0030, 8),
            Err(Setting::DmaLength),
            "a loop that holds bytes, as soon as it holds more than the transfer"
        );
    }

    #[test]
    fn the_owner_check_refuses_a_descriptor_the_cpu_owns() {
        let mut memory = Memory::default();
        memory.descriptor(0x3fce_0000, 4, true, 0x3fca_0000, 0);
        memory.0.insert(0x3fce_0000, SUC_EOF | 4 << 12 | 4);
        let mut gdma = Gdma::new();
        start(&mut gdma, 0x3fce_0000, SPI2);
        assert_eq!(gdma.send(SPI2, 4, memory.byte()).map(|d| d.len()), Ok(4));
        start(&mut gdma, 0x3fce_0000, SPI2);
        gdma.write(CH + OUT + IN_CONF1, CHECK_OWNER).unwrap();
        assert_eq!(
            gdma.send(SPI2, 4, memory.byte()),
            Err(Setting::DmaDescriptor)
        );
    }

    #[test]
    fn only_a_started_channel_on_the_peripheral_feeds_it_and_a_reset_or_stop_drops_the_link() {
        let mut memory = Memory::default();
        memory.descriptor(0x3fce_0000, 1, true, 0x3fca_0000, 0);
        let mut gdma = Gdma::new();
        start(&mut gdma, 0x3fce_0000, sha::PERIPHERAL);
        assert_eq!(
            gdma.send(SPI2, 1, memory.byte()),
            Err(Setting::DmaLink),
            "the SHA accelerator's"
        );
        gdma.write(CH + OUT + IN_LINK, OUTLINK_STOP).unwrap();
        for drop in [(OUT, RST), (OUT + IN_LINK, OUTLINK_STOP)] {
            start(&mut gdma, 0x3fce_0000, SPI2);
            gdma.write(CH + drop.0, drop.1).unwrap();
            assert_eq!(
                gdma.send(SPI2, 1, memory.byte()),
                Err(Setting::DmaLink),
                "{drop:x?}"
            );
        }
        start(&mut gdma, 0x3fce_0000, SPI2);
        gdma.write(OUT + IN_PERI_SEL, SPI2).unwrap();
        gdma.write(OUT + IN_LINK, OUTLINK_START).unwrap();
        assert_eq!(
            gdma.send(SPI2, 1, memory.byte()),
            Err(Setting::DmaLink),
            "two channels"
        );
    }

    #[test]
    fn a_link_the_model_does_not_feed_or_a_peri_sel_change_while_started_is_refused() {
        let mut gdma = Gdma::new();
        for peripheral in [2, 3, 5, 0x3f] {
            gdma.write(CH + OUT + IN_PERI_SEL, peripheral).unwrap();
            assert_eq!(
                gdma.write(CH + OUT + IN_LINK, 0x1234 | OUTLINK_START),
                Err("link_peripheral"),
                "{peripheral}"
            );
            assert_eq!(gdma.read(CH + OUT + IN_LINK), Some(OUTLINK_PARK));
        }
        let list = 0xe_0000 | INLINK_START;
        gdma.write(CH + IN_PERI_SEL, 6).unwrap();
        assert_eq!(
            gdma.write(CH + IN_LINK, list),
            Err("link_peripheral"),
            "AES"
        );
        assert_eq!(gdma.read(CH + IN_LINK), Some(0x0110_0000));
        assert_eq!(
            gdma.write(CH + IN_LINK, INLINK_START),
            Ok(true),
            "AES with no descriptor"
        );
        for spi in [SPI2, SPI3] {
            assert_eq!(gdma.write(CH + IN_PERI_SEL, spi), Ok(true), "no list");
            assert_eq!(gdma.write(CH + IN_LINK, list), Ok(true), "SPI {spi}");
            assert_eq!(
                gdma.write(CH + IN_PERI_SEL, 3),
                Err("peri_sel_while_started")
            );
            for drop in [(IN_LINK, INLINK_STOP), (IN_CONF0, RST)] {
                gdma.write(CH + drop.0, drop.1).unwrap();
                assert_eq!(gdma.write(CH + IN_PERI_SEL, 3), Ok(true), "{drop:x?}");
                gdma.write(CH + IN_PERI_SEL, spi).unwrap();
                gdma.write(CH + IN_LINK, list).unwrap();
            }
            gdma.write(CH + IN_LINK, INLINK_STOP).unwrap();
        }
        start(&mut gdma, 0x3fce_0000, SPI2);
        assert_eq!(
            gdma.write(CH + OUT + IN_PERI_SEL, sha::PERIPHERAL),
            Err("peri_sel_while_started")
        );
        assert_eq!(gdma.write(CH + OUT + IN_PERI_SEL, SPI2), Ok(true));
        gdma.write(CH + OUT + IN_LINK, OUTLINK_STOP).unwrap();
        start(&mut gdma, 0x3fce_0000, SPI3);
        assert_eq!(
            gdma.read(CH + OUT + IN_LINK),
            Some(0xe_0000),
            "SPI3 running"
        );
        gdma.write(CH + OUT + IN_LINK, OUTLINK_STOP).unwrap();
        start(&mut gdma, 0x3fce_0000, sha::PERIPHERAL);
        assert_eq!(gdma.read(CH + OUT + IN_LINK), Some(0xe_0000), "running");
    }

    #[test]
    fn the_registers_reset_as_the_header_says_and_keep_their_fields() {
        let gdma = Gdma::new();
        for (offset, value) in RESET {
            assert_eq!(gdma.read(4 * 0xc0 + offset), Some(value), "{offset:#x}");
        }
        assert_eq!(gdma.read(5 * 0xc0), None);
        assert_eq!(gdma.read(0x18), None, "INFIFO_STATUS is not kept");
        let mut gdma = Gdma::new();
        gdma.write(OUT, 0x30).unwrap();
        gdma.write(OUT + IN_INT_ENA, 0x3).unwrap();
        assert_eq!(gdma.read(OUT), Some(0x30));
        assert_eq!(gdma.read(OUT + IN_INT_ST), Some(0), "no interrupt rises");
        gdma.write(IN_LINK, 0x1234 | INLINK_RESTART).unwrap();
        assert_eq!(gdma.read(IN_LINK), Some(0x1234 | INLINK_PARK));
    }
}
