//! The ESP32-S3 SHA accelerator: SHA-1, SHA-224, SHA-256, SHA-384, SHA-512, SHA-512/224,
//! SHA-512/256 and SHA-512/t, a block at a time from `SHA_M_MEM` or a list of blocks from a GDMA
//! OUT channel whose `GDMA_PERI_OUT_SEL` picks it (`7`), with its DMA completion interrupt
//! (`ETS_SHA_INTR_SOURCE`, source 78).
//!
//! `SHA_START_REG` and `SHA_DMA_START_REG` hash from the mode's initial value (`SHA-512/t` uses
//! `SHA_START_REG` to derive its initial value from `SHA_T_STRING_REG` and `SHA_T_LENGTH_REG`),
//! and `SHA_CONTINUE_REG` and `SHA_DMA_CONTINUE_REG` resume from what `SHA_H_MEM` holds. The TRM
//! gives no cycle count table for SHA; each hash completes at the write that starts it (0 cycles),
//! so `SHA_BUSY_REG` reads 0 and existing runs keep their exact timing. `SHA_H_MEM` and
//! `SHA_M_MEM` hold their bytes in memory order.
//!
//! The registers come from IDF's `soc/esp32s3/include/soc/hwcrypto_reg.h` and `hal/sha_ll.h`.

use orbe_crypto::sha1;

use crate::crypto::sha as math;

use super::spi2::Setting;

const MODE: u32 = 0x00;
const T_STRING: u32 = 0x04;
const T_LENGTH: u32 = 0x08;
const BLOCK_NUM: u32 = 0x0c;
const START: u32 = 0x10;
const CONTINUE: u32 = 0x14;
const BUSY: u32 = 0x18;
const DMA_START: u32 = 0x1c;
const DMA_CONTINUE: u32 = 0x20;
const CLEAR_IRQ: u32 = 0x24;
/// `SHA_INT_ENA_REG`.
pub(crate) const INT_ENA: u32 = 0x28;
const DATE: u32 = 0x2c;
const H: u32 = 0x40;
const M: u32 = 0x80;
const END: u32 = 0x100;

/// Allowed bits of `SHA_INT_ENA_REG`.
pub(crate) const INT_ENA_ALLOWED: u32 = 1;
/// `ETS_SHA_INTR_SOURCE` in IDF's `soc/esp32s3/include/soc/interrupts.h`.
pub(crate) const SOURCE: u32 = 78;
/// `GDMA_PERI_OUT_SEL`'s value for the SHA accelerator.
pub(crate) const PERIPHERAL: u32 = 7;

const DATE_VALUE: u32 = 0x2019_0402;

/// `SHA_MODE_REG` encodings on ESP32-S3 (`rom/sha.h`).
const SHA1: u32 = 0;
const SHA224: u32 = 1;
const SHA256: u32 = 2;
const SHA384: u32 = 3;
const SHA512: u32 = 4;
const SHA512_224: u32 = 5;
const SHA512_256: u32 = 6;
const SHA512_T: u32 = 7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sha {
    mode: u32,
    t_string: u32,
    t_length: u32,
    block_num: u32,
    int_ena: bool,
    int_raw: bool,
    h: [u32; 16],
    m: [u32; 32],
}

impl Sha {
    pub(crate) fn new() -> Sha {
        Sha {
            mode: 0,
            t_string: 0,
            t_length: 0,
            block_num: 0,
            int_ena: false,
            int_raw: false,
            h: [0; 16],
            m: [0; 32],
        }
    }

    /// Whether the SHA interrupt source is asserted.
    pub(crate) fn line(&self) -> bool {
        self.int_raw && self.int_ena
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        Some(match offset {
            MODE => self.mode,
            T_STRING => self.t_string,
            T_LENGTH => self.t_length,
            BLOCK_NUM => self.block_num,
            START | CONTINUE | BUSY | DMA_START | DMA_CONTINUE | CLEAR_IRQ => 0,
            INT_ENA => u32::from(self.int_ena),
            DATE => DATE_VALUE,
            H..M => self.h[((offset - H) / 4) as usize],
            M..END => self.m[((offset - M) / 4) as usize],
            _ => return None,
        })
    }

    /// Writes `value` to the register at `offset`. A write that starts a DMA hash takes
    /// `BLOCK_NUM` blocks from `dma`, which returns that many bytes and latches the SHA interrupt
    /// on completion.
    pub(crate) fn write(
        &mut self,
        offset: u32,
        value: u32,
        dma: impl FnOnce(usize) -> Result<Vec<u8>, Setting>,
    ) -> Result<bool, &'static str> {
        let first = matches!(offset, START | DMA_START);
        match offset {
            MODE => self.mode = value & 0x7,
            T_STRING => self.t_string = value,
            T_LENGTH => self.t_length = value & 0x7f,
            BLOCK_NUM => self.block_num = value,
            INT_ENA => self.int_ena = value & 1 != 0,
            CLEAR_IRQ => {
                if value & 1 != 0 {
                    self.int_raw = false;
                }
            }
            START if value & 1 != 0 && self.mode == SHA512_T => {
                // In SHA-512/t mode, SHA_START_REG computes the initial hash value from T_STRING
                // and T_LENGTH rather than compressing SHA_M_MEM (TRM §18.3.3.3).
                let iv = math::sha512_t_iv(self.t_string, self.t_length);
                self.store_u64_state(iv);
            }
            START | CONTINUE if value & 1 != 0 => {
                let block_len = self.block_bytes();
                let block: Vec<u8> = self.m[..block_len / 4]
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .collect();
                self.hash(first, &block);
            }
            DMA_START | DMA_CONTINUE if value & 1 != 0 => {
                let total = (self.block_num as usize)
                    .checked_mul(self.block_bytes())
                    .ok_or("dma_length")?;
                let bytes = dma(total).map_err(Setting::name)?;
                if first && self.mode == SHA512_T {
                    let iv = math::sha512_t_iv(self.t_string, self.t_length);
                    self.store_u64_state(iv);
                    self.hash(false, &bytes);
                } else {
                    self.hash(first, &bytes);
                }
                // TRM §18.3.4: SHA generates its interrupt in DMA-SHA mode.
                self.int_raw = true;
            }
            START | CONTINUE | DMA_START | DMA_CONTINUE | BUSY | DATE => {}
            H..M => self.h[((offset - H) / 4) as usize] = value,
            M..END => self.m[((offset - M) / 4) as usize] = value,
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn block_bytes(&self) -> usize {
        match self.mode {
            SHA1 | SHA224 | SHA256 => 64,
            _ => 128,
        }
    }

    fn load_u64_state(&self) -> [u64; 8] {
        std::array::from_fn(|i| {
            let hi = u64::from(self.h[2 * i].swap_bytes());
            let lo = u64::from(self.h[2 * i + 1].swap_bytes());
            (hi << 32) | lo
        })
    }

    fn store_u64_state(&mut self, state: [u64; 8]) {
        for (i, word) in state.into_iter().enumerate() {
            self.h[2 * i] = ((word >> 32) as u32).swap_bytes();
            self.h[2 * i + 1] = (word as u32).swap_bytes();
        }
    }

    fn hash(&mut self, first: bool, bytes: &[u8]) {
        match self.mode {
            SHA1 => {
                let mut state = if first {
                    sha1::IV
                } else {
                    std::array::from_fn(|i| self.h[i].swap_bytes())
                };
                sha1::compress(&mut state, bytes);
                for (dst, val) in self.h[..5].iter_mut().zip(state) {
                    *dst = val.swap_bytes();
                }
            }
            SHA224 | SHA256 => {
                let mut state = if first {
                    if self.mode == SHA224 {
                        math::SHA224_IV
                    } else {
                        math::SHA256_IV
                    }
                } else {
                    std::array::from_fn(|i| self.h[i].swap_bytes())
                };
                math::compress_256(&mut state, bytes);
                for (dst, val) in self.h[..8].iter_mut().zip(state) {
                    *dst = val.swap_bytes();
                }
            }
            SHA384 | SHA512 | SHA512_224 | SHA512_256 | SHA512_T => {
                let mut state = if first {
                    match self.mode {
                        SHA384 => math::SHA384_IV,
                        SHA512 => math::SHA512_IV,
                        SHA512_224 => math::SHA512_224_IV,
                        SHA512_256 => math::SHA512_256_IV,
                        _ => self.load_u64_state(),
                    }
                } else {
                    self.load_u64_state()
                };
                math::compress_512(&mut state, bytes);
                self.store_u64_state(state);
            }
            _ => unreachable!("mode is masked to 3 bits"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(sha: &Sha, len: usize) -> Vec<u8> {
        (0..len / 4)
            .flat_map(|i| sha.read(H + 4 * i as u32).unwrap().to_le_bytes())
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn pad64(message: &[u8]) -> Vec<u8> {
        let mut bytes = message.to_vec();
        bytes.push(0x80);
        while bytes.len() % 64 != 56 {
            bytes.push(0);
        }
        bytes.extend((8 * message.len() as u64).to_be_bytes());
        bytes
    }

    fn pad128(message: &[u8]) -> Vec<u8> {
        let mut bytes = message.to_vec();
        bytes.push(0x80);
        while bytes.len() % 128 != 112 {
            bytes.push(0);
        }
        bytes.extend((8 * message.len() as u128).to_be_bytes());
        bytes
    }

    fn no_dma(_: usize) -> Result<Vec<u8>, Setting> {
        panic!("no DMA hash")
    }

    fn hash_blocks(sha: &mut Sha, mode: u32, padded: &[u8], first_is_continue: bool) {
        let block_len = if mode <= SHA256 { 64 } else { 128 };
        sha.write(MODE, mode, no_dma).unwrap();
        for (n, block) in padded.chunks(block_len).enumerate() {
            for (i, word) in block.chunks(4).enumerate() {
                let word = u32::from_le_bytes(word.try_into().unwrap());
                sha.write(M + 4 * i as u32, word, no_dma).unwrap();
            }
            let start = if n == 0 && !first_is_continue {
                START
            } else {
                CONTINUE
            };
            sha.write(start, 1, no_dma).unwrap();
            assert_eq!(sha.read(BUSY), Some(0));
        }
    }

    #[test]
    fn all_sha_modes_match_fips_180_4_in_block_mode() {
        // SHA-1
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA1, &pad64(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 20)),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );

        // SHA-224
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA224, &pad64(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 28)),
            "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7"
        );

        // SHA-256
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA256, &pad64(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 32)),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        // SHA-384
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA384, &pad128(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 48)),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
             8086072ba1e7cc2358baeca134c825a7"
        );

        // SHA-512
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA512, &pad128(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 64)),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );

        // SHA-512/224 & SHA-512/256
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA512_224, &pad128(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 28)),
            "4634270f707b6a54daae7530460842e20e37ed265ceee9a43e8924aa"
        );
        let mut sha = Sha::new();
        hash_blocks(&mut sha, SHA512_256, &pad128(b"abc"), false);
        assert_eq!(
            hex(&digest(&sha, 32)),
            "53048e2681941ef99b2e29b76b4c7dabe4c2d0c634fc6d46e0e2f13107e7af23"
        );

        // SHA-512/t with t = 256 via T_STRING / T_LENGTH + START then CONTINUE
        let mut sha = Sha::new();
        sha.write(MODE, SHA512_T, no_dma).unwrap();
        sha.write(T_STRING, 0x3235_3680, no_dma).unwrap();
        sha.write(T_LENGTH, 0x58, no_dma).unwrap();
        sha.write(START, 1, no_dma).unwrap();
        hash_blocks(&mut sha, SHA512_T, &pad128(b"abc"), true);
        assert_eq!(
            hex(&digest(&sha, 32)),
            "53048e2681941ef99b2e29b76b4c7dabe4c2d0c634fc6d46e0e2f13107e7af23"
        );
    }

    #[test]
    fn dma_sha_continues_across_chunks_and_latches_interrupt() {
        let message: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
        let bytes = pad128(&message);
        let (head, tail) = bytes.split_at(256);

        let mut sha = Sha::new();
        sha.write(MODE, SHA512, no_dma).unwrap();
        sha.write(INT_ENA, 1, no_dma).unwrap();
        assert!(!sha.line());
        sha.write(BLOCK_NUM, 2, no_dma).unwrap();
        sha.write(DMA_START, 1, |n| {
            assert_eq!(n, 256);
            Ok(head.to_vec())
        })
        .unwrap();
        assert!(sha.line(), "DMA-SHA asserts its interrupt");
        sha.write(CLEAR_IRQ, 1, no_dma).unwrap();
        assert!(!sha.line());

        // Save and restore H across a fresh instance to verify context resume for 64-bit SHA.
        let saved: Vec<u32> = (0..16).map(|i| sha.read(H + 4 * i).unwrap()).collect();
        let mut sha = Sha::new();
        sha.write(MODE, SHA512, no_dma).unwrap();
        for (i, word) in saved.iter().enumerate() {
            sha.write(H + 4 * i as u32, *word, no_dma).unwrap();
        }
        sha.write(BLOCK_NUM, (tail.len() / 128) as u32, no_dma)
            .unwrap();
        sha.write(DMA_CONTINUE, 1, |n| {
            assert_eq!(n, tail.len());
            Ok(tail.to_vec())
        })
        .unwrap();

        use sha2::{Digest, Sha512};
        assert_eq!(digest(&sha, 64), Sha512::digest(&message).to_vec());
    }

    #[test]
    fn a_refused_dma_list_leaves_the_digest_and_interrupt_untouched() {
        let mut sha = Sha::new();
        sha.write(MODE, SHA256, no_dma).unwrap();
        sha.write(INT_ENA, 1, no_dma).unwrap();
        sha.write(BLOCK_NUM, 1, no_dma).unwrap();
        let refused = sha.write(DMA_START, 1, |_| Err(Setting::DmaLength));
        assert_eq!(refused, Err("dma_length"));
        assert_eq!(digest(&sha, 32), vec![0; 32]);
        assert!(!sha.line());
    }
}
