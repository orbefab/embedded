//! The ESP32-S3 AES accelerator: AES-128 and AES-256 in Typical AES mode (CPU registers) and
//! DMA-AES mode (ECB, CBC, OFB, CTR with INC32/INC128, CFB8 and CFB128 over GDMA OUT and IN
//! channels), with its completion interrupt (`ETS_AES_INTR_SOURCE`, source 77).
//!
//! Timing (on the APB clock):
//! - Typical AES: ESP32-S3 TRM §19.3.1 gives 11 to 15 cycles for AES-128 and 15 to 19 cycles for
//!   AES-256; this model uses 15 APB cycles for AES-128 and 19 APB cycles for AES-256.
//! - DMA-AES: the TRM gives no per-block cycle table for DMA-AES; this model estimates 15 APB
//!   cycles per AES cipher call for AES-128 and 19 for AES-256 (one call per 16-byte block for
//!   ECB/CBC/OFB/CTR/CFB128, 16 calls per 16-byte block for CFB8).
//!
//! Registers come from IDF's `soc/esp32s3/include/soc/hwcrypto_reg.h` and `hal/aes_ll.h`.

use crate::crypto::aes::{AesKey, BlockMode};

use super::clock::Tick;
use super::gdma::ReceivePlan;

const KEY_BASE: u32 = 0x00;
const KEY_END: u32 = 0x20;
const TEXT_IN_BASE: u32 = 0x20;
const TEXT_IN_END: u32 = 0x30;
const TEXT_OUT_BASE: u32 = 0x30;
const TEXT_OUT_END: u32 = 0x40;
const MODE: u32 = 0x40;
const ENDIAN: u32 = 0x44;
const TRIGGER: u32 = 0x48;
const STATE: u32 = 0x4c;
const IV_BASE: u32 = 0x50;
const IV_END: u32 = 0x60;
const DMA_ENABLE: u32 = 0x90;
const BLOCK_MODE: u32 = 0x94;
const BLOCK_NUM: u32 = 0x98;
const INC_SEL: u32 = 0x9c;
const CONTINUE: u32 = 0xa8;
const INT_CLEAR: u32 = 0xac;
/// `AES_INT_ENA_REG`.
pub(crate) const INT_ENA: u32 = 0xb0;
const DATE: u32 = 0xb4;
const DMA_EXIT: u32 = 0xb8;

/// Allowed bits of `AES_INT_ENA_REG`.
pub(crate) const INT_ENA_ALLOWED: u32 = 1;
/// `ETS_AES_INTR_SOURCE` in IDF's `soc/esp32s3/include/soc/interrupts.h`.
pub(crate) const SOURCE: u32 = 77;
/// `GDMA_PERI_{IN,OUT}_SEL`'s value for the AES accelerator.
pub(crate) const PERIPHERAL: u32 = 6;

const DATE_VALUE: u32 = 0x2019_1210;

/// `AES_STATE_REG` encodings (`hal/aes_ll.h`).
const STATE_IDLE: u32 = 0;
const STATE_BUSY: u32 = 1;
const STATE_DONE: u32 = 2;

/// TRM §19.3.1 Typical AES cycle counts (upper end of 11..=15 and 15..=19 APB cycles).
const CYCLES_AES128: u64 = 15;
const CYCLES_AES256: u64 = 19;

/// Output of a finished DMA-AES operation waiting to be committed to SRAM and GDMA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DmaCompletion {
    pub(crate) out_channel: usize,
    pub(crate) receive: ReceivePlan,
    pub(crate) output: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Typical {
        done_at: Tick,
        output: [u32; 4],
    },
    Dma {
        done_at: Tick,
        next_iv: [u32; 4],
        completion: DmaCompletion,
    },
}

impl Pending {
    fn done_at(&self) -> Tick {
        match self {
            Pending::Typical { done_at, .. } | Pending::Dma { done_at, .. } => *done_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Aes {
    key: [u32; 8],
    text_in: [u32; 4],
    text_out: [u32; 4],
    iv: [u32; 4],
    mode: u32,
    endian: u32,
    dma_enable: bool,
    block_mode: u32,
    block_num: u32,
    inc_sel: bool,
    int_ena: bool,
    int_raw: bool,
    /// Set once a DMA-AES transform finishes, until `AES_DMA_EXIT_REG` is written.
    dma_done: bool,
    pending: Option<Pending>,
}

impl Aes {
    pub(crate) fn new() -> Aes {
        Aes {
            key: [0; 8],
            text_in: [0; 4],
            text_out: [0; 4],
            iv: [0; 4],
            mode: 0,
            endian: 0,
            dma_enable: false,
            block_mode: 0,
            block_num: 0,
            inc_sel: false,
            int_ena: false,
            int_raw: false,
            dma_done: false,
            pending: None,
        }
    }

    /// Whether the AES accelerator is actively executing an operation at `now`.
    pub(crate) fn busy(&self, now: Tick) -> bool {
        self.pending.as_ref().is_some_and(|p| now < p.done_at())
    }

    /// Whether a DMA-AES operation is actively running at `now`.
    pub(crate) fn dma_busy(&self, now: Tick) -> bool {
        matches!(&self.pending, Some(Pending::Dma { done_at, .. }) if now < *done_at)
    }

    /// The tick at which the active operation finishes, if any.
    pub(crate) fn next_event(&self) -> Option<Tick> {
        self.pending.as_ref().map(Pending::done_at)
    }

    /// Whether the AES interrupt source is asserted after settling through `now`.
    pub(crate) fn line(&self) -> bool {
        self.int_raw && self.int_ena
    }

    /// Advances any operation whose completion tick is at or before `now`, returning a finished
    /// DMA transfer for the SoC to write back into SRAM and GDMA.
    pub(crate) fn settle(&mut self, now: Tick) -> Option<DmaCompletion> {
        let ready = self.pending.as_ref().is_some_and(|p| now >= p.done_at());
        if !ready {
            return None;
        }
        match self.pending.take().expect("checked ready") {
            Pending::Typical { output, .. } => {
                self.text_out = output;
                None
            }
            Pending::Dma {
                next_iv,
                completion,
                ..
            } => {
                self.iv = next_iv;
                self.dma_done = true;
                self.int_raw = true;
                Some(completion)
            }
        }
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        Some(match offset {
            KEY_BASE..KEY_END => self.key[((offset - KEY_BASE) / 4) as usize],
            TEXT_IN_BASE..TEXT_IN_END => self.text_in[((offset - TEXT_IN_BASE) / 4) as usize],
            TEXT_OUT_BASE..TEXT_OUT_END => self.text_out[((offset - TEXT_OUT_BASE) / 4) as usize],
            MODE => self.mode,
            ENDIAN => self.endian,
            TRIGGER | CONTINUE | INT_CLEAR | DMA_EXIT => 0,
            STATE => {
                if self.pending.is_some() {
                    STATE_BUSY
                } else if self.dma_done {
                    STATE_DONE
                } else {
                    STATE_IDLE
                }
            }
            IV_BASE..IV_END => self.iv[((offset - IV_BASE) / 4) as usize],
            DMA_ENABLE => u32::from(self.dma_enable),
            BLOCK_MODE => self.block_mode,
            BLOCK_NUM => self.block_num,
            INC_SEL => u32::from(self.inc_sel),
            INT_ENA => u32::from(self.int_ena),
            DATE => DATE_VALUE,
            _ => return None,
        })
    }

    /// Writes `value` at `offset` at tick `now` (with APB cycle length `apb_ticks`).
    ///
    /// On a DMA trigger, `start_dma` receives the byte count and returns the GDMA OUT channel,
    /// input bytes and prepared GDMA IN receive plan.
    pub(crate) fn write(
        &mut self,
        now: Tick,
        apb_ticks: u64,
        offset: u32,
        value: u32,
        start_dma: impl FnOnce(usize) -> Result<(usize, Vec<u8>, ReceivePlan), &'static str>,
    ) -> Result<bool, &'static str> {
        match offset {
            KEY_BASE..KEY_END => self.key[((offset - KEY_BASE) / 4) as usize] = value,
            TEXT_IN_BASE..TEXT_IN_END => {
                self.text_in[((offset - TEXT_IN_BASE) / 4) as usize] = value;
            }
            TEXT_OUT_BASE..TEXT_OUT_END => {
                self.text_out[((offset - TEXT_OUT_BASE) / 4) as usize] = value;
            }
            MODE => self.mode = value & 0x7,
            ENDIAN => {
                if value & 0x3f != 0 {
                    return Err("endian");
                }
                self.endian = 0;
            }
            IV_BASE..IV_END => self.iv[((offset - IV_BASE) / 4) as usize] = value,
            DMA_ENABLE => self.dma_enable = value & 1 != 0,
            BLOCK_MODE => self.block_mode = value & 0x7,
            BLOCK_NUM => self.block_num = value,
            INC_SEL => self.inc_sel = value & 1 != 0,
            INT_ENA => self.int_ena = value & 1 != 0,
            INT_CLEAR => {
                if value & 1 != 0 {
                    self.int_raw = false;
                }
            }
            CONTINUE => {
                if value & 1 != 0 {
                    return Err("gcm");
                }
            }
            DMA_EXIT => {
                // ESP-IDF's `aes_ll_dma_exit` writes 0 here to exit DONE back to IDLE.
                self.dma_done = false;
            }
            TRIGGER if value & 1 != 0 => {
                if self.pending.is_some() {
                    return Ok(true);
                }
                self.start(now, apb_ticks, start_dma)?;
            }
            TRIGGER | STATE | DATE => {}
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn key_and_dir(&self) -> Result<(AesKey, bool, u64), &'static str> {
        let (key_words, encrypt, cycles) = match self.mode {
            0 => (4, true, CYCLES_AES128),
            2 => (8, true, CYCLES_AES256),
            4 => (4, false, CYCLES_AES128),
            6 => (8, false, CYCLES_AES256),
            _ => return Err("mode"),
        };
        let key_bytes: Vec<u8> = self.key[..key_words]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let key = AesKey::new(&key_bytes).expect("16 or 32 bytes");
        Ok((key, encrypt, cycles))
    }

    fn start(
        &mut self,
        now: Tick,
        apb_ticks: u64,
        start_dma: impl FnOnce(usize) -> Result<(usize, Vec<u8>, ReceivePlan), &'static str>,
    ) -> Result<(), &'static str> {
        let (key, encrypt, block_cycles) = self.key_and_dir()?;
        if !self.dma_enable {
            let input = words_to_bytes(&self.text_in);
            let output = if encrypt {
                key.encrypt_block(&input)
            } else {
                key.decrypt_block(&input)
            };
            let done_at = Tick(now.0.saturating_add(block_cycles.saturating_mul(apb_ticks)));
            self.pending = Some(Pending::Typical {
                done_at,
                output: bytes_to_words(&output),
            });
            return Ok(());
        }

        let block_mode = match self.block_mode {
            0 => BlockMode::Ecb,
            1 => BlockMode::Cbc,
            2 => BlockMode::Ofb,
            3 => BlockMode::Ctr {
                inc128: self.inc_sel,
            },
            4 => BlockMode::Cfb8,
            5 => BlockMode::Cfb128,
            _ => return Err("block_mode"),
        };
        if self.block_num == 0 {
            return Err("block_num");
        }
        let bytes = (self.block_num as usize)
            .checked_mul(16)
            .ok_or("dma_length")?;
        let (out_channel, input, receive) = start_dma(bytes)?;
        let mut iv = words_to_bytes(&self.iv);
        let output = key.transform(block_mode, encrypt, &mut iv, &input);
        let calls_per_block = if matches!(block_mode, BlockMode::Cfb8) {
            16
        } else {
            1
        };
        let cycles = u64::from(self.block_num)
            .saturating_mul(calls_per_block)
            .saturating_mul(block_cycles);
        let done_at = Tick(now.0.saturating_add(cycles.saturating_mul(apb_ticks)));
        self.dma_done = false;
        self.pending = Some(Pending::Dma {
            done_at,
            next_iv: bytes_to_words(&iv),
            completion: DmaCompletion {
                out_channel,
                receive,
                output,
            },
        });
        Ok(())
    }
}

fn words_to_bytes(words: &[u32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

fn bytes_to_words(bytes: &[u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_dma(_: usize) -> Result<(usize, Vec<u8>, ReceivePlan), &'static str> {
        panic!("no DMA")
    }

    #[test]
    fn typical_aes_encrypts_and_decrypts_after_trm_cycles() {
        let mut aes = Aes::new();
        let apb = 6;
        // FIPS 197 Appendix C AES-128 vector
        let key: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let pt: [u8; 16] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let want: [u8; 16] = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        for (i, w) in bytes_to_words(&key).into_iter().enumerate() {
            aes.write(Tick(0), apb, KEY_BASE + 4 * i as u32, w, no_dma)
                .unwrap();
        }
        for (i, w) in bytes_to_words(&pt).into_iter().enumerate() {
            aes.write(Tick(0), apb, TEXT_IN_BASE + 4 * i as u32, w, no_dma)
                .unwrap();
        }
        aes.write(Tick(0), apb, MODE, 0, no_dma).unwrap();
        aes.write(Tick(100), apb, TRIGGER, 1, no_dma).unwrap();

        assert_eq!(aes.read(STATE), Some(STATE_BUSY));
        assert_eq!(aes.next_event(), Some(Tick(100 + CYCLES_AES128 * apb)));
        assert!(aes.settle(Tick(100 + CYCLES_AES128 * apb - 1)).is_none());
        assert_eq!(aes.read(STATE), Some(STATE_BUSY));

        assert!(aes.settle(Tick(100 + CYCLES_AES128 * apb)).is_none());
        assert_eq!(aes.read(STATE), Some(STATE_IDLE));
        let got: [u32; 4] =
            std::array::from_fn(|i| aes.read(TEXT_OUT_BASE + 4 * i as u32).unwrap());
        assert_eq!(words_to_bytes(&got), want);
    }

    #[test]
    fn reserved_modes_non_zero_endian_and_gcm_are_refused() {
        let mut aes = Aes::new();
        assert_eq!(aes.write(Tick(0), 6, ENDIAN, 1, no_dma), Err("endian"));
        assert_eq!(aes.write(Tick(0), 6, CONTINUE, 1, no_dma), Err("gcm"));

        // Reserved AES-192 mode (1)
        aes.write(Tick(0), 6, MODE, 1, no_dma).unwrap();
        assert_eq!(aes.write(Tick(0), 6, TRIGGER, 1, no_dma), Err("mode"));

        // Unsupported GCM block mode (6) in DMA mode
        aes.write(Tick(0), 6, MODE, 0, no_dma).unwrap();
        aes.write(Tick(0), 6, DMA_ENABLE, 1, no_dma).unwrap();
        aes.write(Tick(0), 6, BLOCK_MODE, 6, no_dma).unwrap();
        aes.write(Tick(0), 6, BLOCK_NUM, 1, no_dma).unwrap();
        assert_eq!(aes.write(Tick(0), 6, TRIGGER, 1, no_dma), Err("block_mode"));
    }
}
