//! The SHA accelerator, as far as SHA-256 runs on it, a block at a time from its message
//! registers or a list of blocks from a GDMA OUT channel whose `GDMA_PERI_OUT_SEL` picks it.
//!
//! `SHA_START_REG` and `SHA_DMA_START_REG` hash from the SHA-256 initial value,
//! `SHA_CONTINUE_REG` and `SHA_DMA_CONTINUE_REG` from what `SHA_H_MEM` holds. A hash ends at the
//! write that starts it, so `SHA_BUSY_REG` reads 0. `SHA_H_MEM` and `SHA_M_MEM` hold their bytes
//! in memory order, the digest's big-endian words as the CPU copies them out.
//!
//! The registers come from IDF's `soc/esp32s3/include/soc/hwcrypto_reg.h`.

use sha2::compress256;
use sha2::digest::generic_array::GenericArray;

use super::spi2::Setting;

const MODE: u32 = 0x00;
const BLOCK_NUM: u32 = 0x0c;
const START: u32 = 0x10;
const CONTINUE: u32 = 0x14;
const BUSY: u32 = 0x18;
const DMA_START: u32 = 0x1c;
const DMA_CONTINUE: u32 = 0x20;
/// `SHA_INT_ENA_REG`.
pub(crate) const INT_ENA: u32 = 0x28;
const H: u32 = 0x40;
const M: u32 = 0x80;
const END: u32 = 0x100;

/// `SHA_MODE_REG`'s value for SHA-256.
const SHA256: u32 = 2;
const BLOCK: usize = 64;
/// `GDMA_PERI_OUT_SEL`'s value for the SHA accelerator.
pub(crate) const PERIPHERAL: u32 = 7;

const INITIAL: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sha {
    mode: u32,
    block_num: u32,
    h: [u32; 16],
    m: [u32; 32],
}

impl Sha {
    pub(crate) fn new() -> Sha {
        Sha {
            mode: 0,
            block_num: 0,
            h: [0; 16],
            m: [0; 32],
        }
    }

    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        Some(match offset {
            MODE => self.mode,
            BLOCK_NUM => self.block_num,
            START | CONTINUE | BUSY | DMA_START | DMA_CONTINUE => 0,
            H..M => self.h[((offset - H) / 4) as usize],
            M..END => self.m[((offset - M) / 4) as usize],
            _ => return None,
        })
    }

    /// Whether the register at `offset` is kept, after the write of `value` to it. A write that
    /// starts a DMA hash takes `BLOCK_NUM` blocks from `dma`, which gives that many bytes. The
    /// setting that refuses a hash in another mode than SHA-256, or the bytes `dma` refuses.
    pub(crate) fn write(
        &mut self,
        offset: u32,
        value: u32,
        dma: impl FnOnce(usize) -> Result<Vec<u8>, Setting>,
    ) -> Result<bool, &'static str> {
        let first = matches!(offset, START | DMA_START);
        match offset {
            MODE => self.mode = value & 0x7,
            BLOCK_NUM => self.block_num = value,
            START | CONTINUE if value & 1 != 0 => {
                let block: Vec<u8> = self.m[..BLOCK / 4]
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .collect();
                self.hash(first, &block)?;
            }
            DMA_START | DMA_CONTINUE if value & 1 != 0 => {
                if self.mode != SHA256 {
                    return Err("mode");
                }
                let bytes = dma(self.block_num as usize * BLOCK).map_err(Setting::name)?;
                self.hash(first, &bytes)?;
            }
            START | CONTINUE | DMA_START | DMA_CONTINUE | BUSY => {}
            H..M => self.h[((offset - H) / 4) as usize] = value,
            M..END => self.m[((offset - M) / 4) as usize] = value,
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn hash(&mut self, first: bool, bytes: &[u8]) -> Result<(), &'static str> {
        if self.mode != SHA256 {
            return Err("mode");
        }
        let mut state = if first {
            INITIAL
        } else {
            std::array::from_fn(|i| self.h[i].swap_bytes())
        };
        let blocks: Vec<_> = bytes
            .as_chunks::<BLOCK>()
            .0
            .iter()
            .map(|block| GenericArray::from(*block))
            .collect();
        compress256(&mut state, &blocks);
        for (word, value) in self.h.iter_mut().zip(state) {
            *word = value.swap_bytes();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(sha: &Sha) -> Vec<u8> {
        (0..8)
            .flat_map(|i| sha.read(H + 4 * i).unwrap().to_le_bytes())
            .collect()
    }

    fn padded(message: &[u8]) -> Vec<u8> {
        let mut bytes = message.to_vec();
        bytes.push(0x80);
        while bytes.len() % BLOCK != BLOCK - 8 {
            bytes.push(0);
        }
        bytes.extend((8 * message.len() as u64).to_be_bytes());
        bytes
    }

    fn reference(message: &[u8]) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        Sha256::digest(message).to_vec()
    }

    fn no_dma(_: usize) -> Result<Vec<u8>, Setting> {
        panic!("no DMA hash")
    }

    #[test]
    fn blocks_from_the_message_registers_hash_to_the_sha256_digest() {
        let message: Vec<u8> = (0..100u8).collect();
        let mut sha = Sha::new();
        sha.write(MODE, SHA256, no_dma).unwrap();
        for (n, block) in padded(&message).chunks(BLOCK).enumerate() {
            for (i, word) in block.chunks(4).enumerate() {
                let word = u32::from_le_bytes(word.try_into().unwrap());
                sha.write(M + 4 * i as u32, word, no_dma).unwrap();
            }
            let start = if n == 0 { START } else { CONTINUE };
            sha.write(start, 1, no_dma).unwrap();
            assert_eq!(sha.read(BUSY), Some(0));
        }
        assert_eq!(digest(&sha), reference(&message));
    }

    #[test]
    fn a_dma_hash_takes_block_num_blocks_and_continues_from_the_digest_registers() {
        let message: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
        let bytes = padded(&message);
        let (head, tail) = bytes.split_at(2 * BLOCK);
        let mut sha = Sha::new();
        sha.write(MODE, SHA256, no_dma).unwrap();
        sha.write(BLOCK_NUM, 2, no_dma).unwrap();
        sha.write(DMA_START, 1, |n| {
            assert_eq!(n, 2 * BLOCK);
            Ok(head.to_vec())
        })
        .unwrap();
        let saved: Vec<u32> = (0..8).map(|i| sha.read(H + 4 * i).unwrap()).collect();
        let mut sha = Sha::new();
        sha.write(MODE, SHA256, no_dma).unwrap();
        for (i, word) in saved.iter().enumerate() {
            sha.write(H + 4 * i as u32, *word, no_dma).unwrap();
        }
        sha.write(BLOCK_NUM, (tail.len() / BLOCK) as u32, no_dma)
            .unwrap();
        sha.write(DMA_CONTINUE, 1, |n| {
            assert_eq!(n, tail.len());
            Ok(tail.to_vec())
        })
        .unwrap();
        assert_eq!(digest(&sha), reference(&message));
    }

    #[test]
    fn another_mode_or_a_refused_dma_list_refuses_the_hash() {
        let mut sha = Sha::new();
        sha.write(MODE, 0, no_dma).unwrap();
        assert_eq!(sha.write(START, 1, no_dma), Err("mode"));
        sha.write(BLOCK_NUM, 1, no_dma).unwrap();
        assert_eq!(sha.write(DMA_START, 1, no_dma), Err("mode"));
        sha.write(MODE, SHA256, no_dma).unwrap();
        let refused = sha.write(DMA_START, 1, |_| Err(Setting::DmaLength));
        assert_eq!(refused, Err("dma_length"));
        assert_eq!(digest(&sha), vec![0; 32]);
    }
}
