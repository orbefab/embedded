//! GDMA, the DMA controller of five channel pairs.
//!
//! OUT channels read their descriptor lists from data-bus SRAM for SPI2 (0), AES (6) and SHA (7).
//! IN channels write peripheral output into data-bus SRAM for AES (6), updating each consumed
//! descriptor in SRAM (`owner = 0`, `length`, `suc_eof`) and `GDMA_IN_SUC_EOF_DES_ADDR_CHn_REG`.
//!
//! A descriptor is three words: `size` in bits 0 to 11, `length` in bits 12 to 23, `suc_eof` in
//! bit 30 and `owner` in bit 31, then the buffer's address, then the next descriptor's.
//! `GDMA_{IN,OUT}LINK_ADDR` holds the low 20 bits of the first descriptor's address in the
//! data-bus SRAM window (`0x3fc8_8000..0x3fd0_0000`).
//!
//! An IN link started at a non-SRAM address (such as address 0, which IDF's shared crypto DMA
//! writes to the AES IN channel beside each SHA hash) parks without starting. Starting an OUT
//! link or an SRAM IN link for an unmodeled peripheral, or changing a channel's `peri_sel` while
//! its link is started, ends the run.
//!
//! Each channel pair raises its IN interrupt (`ETS_DMA_IN_CHn_INTR_SOURCE`, 66..=70) on `IN_DONE`
//! / `IN_SUC_EOF` and its OUT interrupt (`ETS_DMA_OUT_CHn_INTR_SOURCE`, 71..=75) on `OUT_DONE` /
//! `OUT_EOF` / `OUT_TOTAL_EOF`. Registers come from IDF's
//! `soc/esp32s3/register/soc/gdma_reg.h` and `gdma_struct.h`.

use std::ops::Range;

use super::aes;
use super::sha;
use super::spi2::Setting;

pub(crate) const CHANNELS: usize = 5;
/// The words of one channel pair, `GDMA_IN_CONF0_CHn_REG` to `GDMA_OUT_PERI_SEL_CHn_REG`.
const WORDS: usize = 0xc0 / 4;

const IN_CONF0: u32 = 0x00;
const IN_CONF1: u32 = 0x04;
const IN_INT_RAW: u32 = 0x08;
const IN_INT_ST: u32 = 0x0c;
const IN_INT_ENA: u32 = 0x10;
const IN_INT_CLR: u32 = 0x14;
const IN_LINK: u32 = 0x20;
const IN_SUC_EOF_DES_ADDR: u32 = 0x28;
const IN_DSCR: u32 = 0x30;
const IN_WEIGHT: u32 = 0x3c;
const IN_PRI: u32 = 0x44;
const IN_PERI_SEL: u32 = 0x48;
/// The OUT registers sit this far above their IN twins.
const OUT: u32 = 0x60;

const RST: u32 = 1 << 0;
/// `GDMA_OUT_AUTO_WRBACK`, which writes each OUT descriptor back once its buffer is sent.
pub(crate) const AUTO_WRBACK: u32 = 1 << 2;
/// `GDMA_MEM_TRANS_EN`, which copies memory to memory.
pub(crate) const MEM_TRANS_EN: u32 = 1 << 4;
/// `GDMA_{IN,OUT}_CHECK_OWNER`.
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

/// GDMA IN interrupt bits.
const IN_DONE: u32 = 1 << 0;
const IN_SUC_EOF: u32 = 1 << 1;
/// `IN_ERR_EOF` (UHCI0 only; IDF enables it alongside `IN_SUC_EOF` on `on_recv_eof`).
const IN_ERR_EOF: u32 = 1 << 2;
/// Allowed bits of `GDMA_IN_INT_ENA_CHn_REG`.
pub(crate) const IN_INT_ENA_ALLOWED: u32 = IN_DONE | IN_SUC_EOF | IN_ERR_EOF;

/// GDMA OUT interrupt bits.
const OUT_DONE: u32 = 1 << 0;
const OUT_EOF: u32 = 1 << 1;
const OUT_TOTAL_EOF: u32 = 1 << 3;
/// Allowed bits of `GDMA_OUT_INT_ENA_CHn_REG`.
pub(crate) const OUT_INT_ENA_ALLOWED: u32 = OUT_DONE | OUT_EOF | OUT_TOTAL_EOF;

/// Interrupt matrix sources: `ETS_DMA_IN_CH0_INTR_SOURCE`..=CH4 and
/// `ETS_DMA_OUT_CH0_INTR_SOURCE`..=CH4.
pub(crate) const IN_SOURCE0: u32 = 66;
pub(crate) const OUT_SOURCE0: u32 = 71;

/// `GDMA_PERI_{IN,OUT}_SEL`'s value for SPI2.
pub(crate) const SPI2: u32 = 0;

/// The data-bus SRAM window, where a descriptor and its buffer lie.
pub(crate) const DMA_RAM: Range<u32> = 0x3fc8_8000..0x3fd0_0000;
/// The high bits `GDMA_{IN,OUT}LINK_ADDR` leaves out.
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

/// A validated GDMA IN writeback plan prepared when a peripheral starts and committed when it
/// completes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReceivePlan {
    pub(crate) channel: usize,
    pub(crate) eof_desc: u32,
    /// `(buffer_addr, byte_len)` slices to fill in order from the peripheral's output.
    pub(crate) buffers: Vec<(u32, usize)>,
    /// `(descriptor_addr, new_dw0)` descriptor writebacks (`owner = 0`, `length`, `suc_eof`).
    pub(crate) writebacks: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Gdma {
    regs: [[u32; WORDS]; CHANNELS],
    /// The descriptor each OUT channel's link started at, until a transfer takes it, or a stop
    /// or a channel reset drops it.
    started: [Option<u32>; CHANNELS],
    /// The descriptor each IN channel's link started at in SRAM, until a transfer completes, or a
    /// stop or a channel reset drops it.
    in_started: [Option<u32>; CHANNELS],
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
            in_started: [None; CHANNELS],
        }
    }

    /// Interrupt source bitmask for GDMA IN CH0..CH4 and OUT CH0..CH4.
    pub(crate) fn interrupt_sources(&self) -> u128 {
        let mut mask = 0u128;
        for ch in 0..CHANNELS {
            let regs = &self.regs[ch];
            let in_st = regs[(IN_INT_RAW / 4) as usize] & regs[(IN_INT_ENA / 4) as usize];
            if in_st != 0 {
                mask |= 1u128 << (IN_SOURCE0 + ch as u32);
            }
            let out_st =
                regs[((OUT + IN_INT_RAW) / 4) as usize] & regs[((OUT + IN_INT_ENA) / 4) as usize];
            if out_st != 0 {
                mask |= 1u128 << (OUT_SOURCE0 + ch as u32);
            }
        }
        mask
    }

    /// The channel and the offset in its pair of a kept register at `offset`.
    fn kept(offset: u32) -> Option<(usize, u32)> {
        let channel = (offset / 0xc0) as usize;
        let at = offset % 0xc0;
        let local = at % OUT;
        let kept = matches!(
            local,
            IN_CONF0
                ..=IN_INT_CLR
                    | IN_LINK
                    | IN_SUC_EOF_DES_ADDR
                    | IN_DSCR
                    | IN_WEIGHT
                    | IN_PRI
                    | IN_PERI_SEL
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
            IN_LINK => {
                let park = if self.in_started[channel].is_some() {
                    0
                } else {
                    INLINK_PARK
                };
                (word(at) & !(INLINK_STROBES | INLINK_PARK)) | park
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
            let fed = &[SPI2, aes::PERIPHERAL, sha::PERIPHERAL];
            (OUTLINK_START, fed, self.started[channel].is_some())
        } else {
            let fed = &[SPI2, aes::PERIPHERAL];
            (INLINK_START, fed, self.in_started[channel].is_some())
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
                        self.in_started[channel] = None;
                    }
                }
            }
            IN_INT_RAW | IN_INT_ST | IN_SUC_EOF_DES_ADDR | IN_DSCR => {}
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
                *word = value & !(INLINK_STROBES | INLINK_PARK);
                if value & INLINK_STOP != 0 {
                    self.in_started[channel] = None;
                }
                if value & INLINK_START != 0 {
                    self.in_started[channel] = listed.then_some(LINK_BASE | value & OUTLINK_ADDR);
                }
            }
            _ => *word = value,
        }
        Ok(true)
    }

    /// Reads `bytes` bytes from the unique OUT channel started for `peripheral` and immediately
    /// latches its OUT completion interrupts. Used for synchronous OUT consumers (SPI2, SHA).
    pub(crate) fn send(
        &mut self,
        peripheral: u32,
        bytes: usize,
        byte: impl FnMut(u32) -> u8,
    ) -> Result<Vec<u8>, Setting> {
        let (channel, data) = self.read_out_list(peripheral, bytes, byte)?;
        self.finish_send(channel);
        Ok(data)
    }

    /// Reads `bytes` bytes from the unique OUT channel started for `peripheral` without yet
    /// latching its OUT interrupts, returning `(channel, data)` so an asynchronous peripheral
    /// (such as AES DMA) can latch completion when it finishes.
    pub(crate) fn read_out_list(
        &mut self,
        peripheral: u32,
        bytes: usize,
        mut byte: impl FnMut(u32) -> u8,
    ) -> Result<(usize, Vec<u8>), Setting> {
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
            if !at.is_multiple_of(4) || !inside_sram(at, 12) {
                return Err(Setting::DmaDescriptor);
            }
            let [head, buffer, next] = [at, at + 4, at + 8]
                .map(|word| u32::from_le_bytes([0, 1, 2, 3].map(|k| byte(word + k))));
            let length = (head >> 12) & 0xfff;
            if (check_owner && head & OWNER == 0) || (length > 0 && !inside_sram(buffer, length)) {
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
                self.regs[channel][((OUT + IN_SUC_EOF_DES_ADDR) / 4) as usize] = at;
                self.regs[channel][((OUT + IN_DSCR) / 4) as usize] = next;
                return Ok((channel, data));
            }
            at = next;
        }
        Err(Setting::DmaDescriptor)
    }

    /// Latches OUT completion interrupts for `channel`.
    pub(crate) fn finish_send(&mut self, channel: usize) {
        self.regs[channel][((OUT + IN_INT_RAW) / 4) as usize] |= OUT_DONE | OUT_EOF | OUT_TOTAL_EOF;
    }

    /// Validates the unique IN channel started for `peripheral` and builds a [`ReceivePlan`] for
    /// writing `bytes` bytes into its SRAM descriptor list. Leaves the channel active until
    /// [`Self::finish_receive`] is called.
    pub(crate) fn prepare_receive(
        &self,
        peripheral: u32,
        bytes: usize,
        mut byte: impl FnMut(u32) -> u8,
    ) -> Result<ReceivePlan, Setting> {
        let mut receiving = (0..CHANNELS).filter(|&channel| {
            self.in_started[channel].is_some()
                && self.regs[channel][(IN_PERI_SEL / 4) as usize] & 0x3f == peripheral
        });
        let (Some(channel), None) = (receiving.next(), receiving.next()) else {
            return Err(Setting::DmaLink);
        };
        let check_owner = self.regs[channel][(IN_CONF1 / 4) as usize] & CHECK_OWNER != 0;
        let mut at = self.in_started[channel].expect("in_started");
        let mut remaining = bytes;
        let mut buffers = Vec::new();
        let mut writebacks = Vec::new();

        for _ in 0..=MOST_DESCRIPTORS {
            if !at.is_multiple_of(4) || !inside_sram(at, 12) {
                return Err(Setting::DmaDescriptor);
            }
            let [head, buffer, next] = [at, at + 4, at + 8]
                .map(|word| u32::from_le_bytes([0, 1, 2, 3].map(|k| byte(word + k))));
            let size = (head & 0xfff) as usize;
            if (check_owner && head & OWNER == 0)
                || (size == 0 && remaining > 0)
                || (size > 0 && !inside_sram(buffer, size as u32))
            {
                return Err(Setting::DmaDescriptor);
            }
            let take = remaining.min(size);
            remaining -= take;
            buffers.push((buffer, take));
            let is_eof = remaining == 0;
            // Descriptor writeback: clear OWNER, write received `length`, set SUC_EOF on last.
            let cleared = head & !(OWNER | SUC_EOF | (0xfff << 12));
            let new_head = cleared | ((take as u32) << 12) | if is_eof { SUC_EOF } else { 0 };
            writebacks.push((at, new_head));
            if is_eof {
                return Ok(ReceivePlan {
                    channel,
                    eof_desc: at,
                    buffers,
                    writebacks,
                });
            }
            if next == 0 {
                return Err(Setting::DmaLength);
            }
            at = next;
        }
        Err(Setting::DmaDescriptor)
    }

    /// Marks `plan`'s IN channel as parked, records `IN_SUC_EOF_DES_ADDR`, and latches
    /// `IN_DONE | IN_SUC_EOF`.
    pub(crate) fn finish_receive(&mut self, plan: &ReceivePlan) {
        let ch = plan.channel;
        self.in_started[ch] = None;
        self.regs[ch][(IN_SUC_EOF_DES_ADDR / 4) as usize] = plan.eof_desc;
        self.regs[ch][(IN_DSCR / 4) as usize] = plan.eof_desc;
        self.regs[ch][(IN_INT_RAW / 4) as usize] |= IN_DONE | IN_SUC_EOF;
    }
}

fn inside_sram(addr: u32, len: u32) -> bool {
    DMA_RAM.contains(&addr) && u64::from(addr) + u64::from(len) <= u64::from(DMA_RAM.end)
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

        fn get(&self, addr: u32, len: usize) -> Vec<u8> {
            (addr..addr + len as u32).map(self.byte()).collect()
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
        assert_eq!(gdma.read(CH + OUT + IN_SUC_EOF_DES_ADDR), Some(0x3fce_a600));
        assert_eq!(
            gdma.read(CH + OUT + IN_INT_RAW),
            Some(OUT_DONE | OUT_EOF | OUT_TOTAL_EOF)
        );
        assert_eq!(gdma.send(SPI2, 11, memory.byte()), Err(Setting::DmaLink));
    }

    #[test]
    fn in_channel_writes_back_descriptors_and_raises_interrupt() {
        let mut memory = Memory::default();
        memory.descriptor(0x3fce_0000, 10, false, 0x3fca_0000, 0x3fce_0010);
        memory.descriptor(0x3fce_0010, 16, false, 0x3fcb_0000, 0);
        let mut gdma = Gdma::new();
        gdma.write(CH + IN_PERI_SEL, aes::PERIPHERAL).unwrap();
        gdma.write(CH + IN_INT_ENA, IN_DONE | IN_SUC_EOF).unwrap();
        gdma.write(CH + IN_LINK, 0xe_0000 | INLINK_START).unwrap();
        assert_eq!(gdma.read(CH + IN_LINK), Some(0xe_0000), "not parked");
        assert_eq!(gdma.interrupt_sources(), 0);

        let payload = b"0123456789abcdef";
        let plan = gdma
            .prepare_receive(aes::PERIPHERAL, payload.len(), memory.byte())
            .unwrap();
        let mut pos = 0;
        for &(buf, len) in &plan.buffers {
            memory.put(buf, &payload[pos..pos + len]);
            pos += len;
        }
        for &(desc, head) in &plan.writebacks {
            memory.0.insert(desc, head);
        }
        gdma.finish_receive(&plan);

        assert_eq!(memory.get(0x3fca_0000, 10), &payload[..10]);
        assert_eq!(memory.get(0x3fcb_0000, 6), &payload[10..]);
        // First descriptor: owner=0, suc_eof=0, length=10, size=10
        assert_eq!(memory.0[&0x3fce_0000], (10 << 12) | 10);
        // Second descriptor: owner=0, suc_eof=1, length=6, size=16
        assert_eq!(memory.0[&0x3fce_0010], SUC_EOF | (6 << 12) | 16);
        assert_eq!(gdma.read(CH + IN_LINK), Some(0xe_0000 | INLINK_PARK));
        assert_eq!(gdma.read(CH + IN_SUC_EOF_DES_ADDR), Some(0x3fce_0010));
        assert_eq!(gdma.read(CH + IN_INT_ST), Some(IN_DONE | IN_SUC_EOF));
        assert_eq!(gdma.interrupt_sources(), 1u128 << (IN_SOURCE0 + 2));

        gdma.write(CH + IN_INT_CLR, IN_DONE | IN_SUC_EOF).unwrap();
        assert_eq!(gdma.interrupt_sources(), 0);
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
        for peripheral in [1, 3, 5, 0x3f] {
            gdma.write(CH + OUT + IN_PERI_SEL, peripheral).unwrap();
            assert_eq!(
                gdma.write(CH + OUT + IN_LINK, 0x1234 | OUTLINK_START),
                Err("link_peripheral"),
                "{peripheral}"
            );
            assert_eq!(gdma.read(CH + OUT + IN_LINK), Some(OUTLINK_PARK));
        }
        let list = 0xe_0000 | INLINK_START;
        gdma.write(CH + IN_PERI_SEL, 3).unwrap();
        assert_eq!(
            gdma.write(CH + IN_LINK, list),
            Err("link_peripheral"),
            "I2S0"
        );
        assert_eq!(gdma.read(CH + IN_LINK), Some(0x0110_0000));
        gdma.write(CH + IN_PERI_SEL, aes::PERIPHERAL).unwrap();
        assert_eq!(
            gdma.write(CH + IN_LINK, INLINK_START),
            Ok(true),
            "AES with no descriptor"
        );
        assert_eq!(
            gdma.write(CH + IN_LINK, list),
            Ok(true),
            "AES with SRAM descriptor"
        );
        assert_eq!(
            gdma.write(CH + IN_PERI_SEL, SPI2),
            Err("peri_sel_while_started")
        );
        gdma.write(CH + IN_LINK, INLINK_STOP).unwrap();
        assert_eq!(gdma.write(CH + IN_PERI_SEL, SPI2), Ok(true), "no list");
        assert_eq!(gdma.write(CH + IN_LINK, list), Ok(true), "SPI2's");
        assert_eq!(
            gdma.write(CH + IN_PERI_SEL, 3),
            Err("peri_sel_while_started")
        );
        for drop in [(IN_LINK, INLINK_STOP), (IN_CONF0, RST)] {
            gdma.write(CH + drop.0, drop.1).unwrap();
            assert_eq!(gdma.write(CH + IN_PERI_SEL, 3), Ok(true), "{drop:x?}");
            gdma.write(CH + IN_PERI_SEL, SPI2).unwrap();
            gdma.write(CH + IN_LINK, list).unwrap();
        }
        start(&mut gdma, 0x3fce_0000, SPI2);
        assert_eq!(
            gdma.write(CH + OUT + IN_PERI_SEL, sha::PERIPHERAL),
            Err("peri_sel_while_started")
        );
        assert_eq!(gdma.write(CH + OUT + IN_PERI_SEL, SPI2), Ok(true));
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
        assert_eq!(gdma.read(OUT + IN_INT_ST), Some(0));
        gdma.write(IN_LINK, 0x1234 | INLINK_RESTART).unwrap();
        assert_eq!(gdma.read(IN_LINK), Some(0x1234 | INLINK_PARK));
    }
}
