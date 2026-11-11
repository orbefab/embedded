use sha2::compress256;
use sha2::digest::generic_array::GenericArray;

/// SHA-256 of bytes fed in parts, as plain data, so a device can keep one in its state. A screen
/// hashes each pixel write with it, and the trace shows the first 8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    /// How many bytes of `block` are filled, below 64.
    filled: u8,
    /// Bytes fed so far.
    len: u64,
}

const INIT: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    pub const fn new() -> Sha256 {
        Sha256 {
            state: INIT,
            block: [0; 64],
            filled: 0,
            len: 0,
        }
    }

    pub fn update(&mut self, mut bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        let filled = usize::from(self.filled);
        if filled > 0 {
            let take = bytes.len().min(64 - filled);
            self.block[filled..filled + take].copy_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if filled + take < 64 {
                self.filled = (filled + take) as u8;
                return;
            }
            compress(&mut self.state, &self.block);
        }
        let (blocks, rest) = bytes.as_chunks::<64>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        self.block[..rest.len()].copy_from_slice(rest);
        self.filled = rest.len() as u8;
    }

    /// The digest of every byte fed.
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        let filled = usize::from(self.filled);
        self.block[filled] = 0x80;
        self.block[filled + 1..].fill(0);
        if filled >= 56 {
            compress(&mut self.state, &self.block);
            self.block.fill(0);
        }
        self.block[56..].copy_from_slice(&bits.to_be_bytes());
        compress(&mut self.state, &self.block);
        let mut digest = [0; 32];
        for (out, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            out.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    compress256(state, std::slice::from_ref(GenericArray::from_slice(block)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    #[test]
    fn it_matches_sha2_over_any_split() {
        let bytes: Vec<u8> = (0..300u32).map(|k| (k * 37 % 251) as u8).collect();
        for len in [0, 1, 55, 56, 63, 64, 65, 119, 120, 128, 300] {
            let whole = sha2::Sha256::digest(&bytes[..len]);
            for split in [0, 1, 7, 63, 64, 65] {
                let split = split.min(len);
                let mut hash = Sha256::new();
                hash.update(&bytes[..split]);
                hash.update(&bytes[split..len]);
                assert_eq!(hash.finish()[..], whole[..], "len {len}, split {split}");
            }
        }
    }
}
