//! SHA-1 (FIPS 180-4 §6.1): the compression function the ESP32-S3's SHA accelerator runs, and
//! the hash HMAC-SHA1 runs over.

/// Initial hash value for SHA-1 (FIPS 180-4 §5.3.1).
pub const IV: [u32; 5] = [
    0x6745_2301,
    0xefcd_ab89,
    0x98ba_dcfe,
    0x1032_5476,
    0xc3d2_e1f0,
];

/// Compresses one or more 64-byte blocks with SHA-1 (FIPS 180-4 §6.1.2).
pub fn compress(state: &mut [u32; 5], bytes: &[u8]) {
    let (blocks, remainder) = bytes.as_chunks::<64>();
    debug_assert!(remainder.is_empty());
    for block in blocks {
        let mut w = [0u32; 80];
        for (dst, chunk) in w[..16].iter_mut().zip(block.as_chunks::<4>().0) {
            *dst = u32::from_be_bytes(*chunk);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = *state;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | (!b & d), 0x5a82_7999u32),
                20..40 => (b ^ c ^ d, 0x6ed9_eba1u32),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdcu32),
                _ => (b ^ c ^ d, 0xca62_c1d6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
}

/// SHA-1 over a message given in any number of pieces.
#[derive(Clone, Debug)]
pub struct Sha1 {
    state: [u32; 5],
    block: [u8; 64],
    filled: usize,
    len: u64,
}

impl Default for Sha1 {
    fn default() -> Sha1 {
        Sha1::new()
    }
}

impl Sha1 {
    pub const fn new() -> Sha1 {
        Sha1 {
            state: IV,
            block: [0; 64],
            filled: 0,
            len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.filled > 0 {
            let take = data.len().min(64 - self.filled);
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled < 64 {
                return;
            }
            compress(&mut self.state, &self.block);
            self.filled = 0;
        }
        let whole = data.len() - data.len() % 64;
        compress(&mut self.state, &data[..whole]);
        let rest = &data[whole..];
        self.block[..rest.len()].copy_from_slice(rest);
        self.filled = rest.len();
    }

    /// The digest, after the padding of FIPS 180-4 §5.1.1.
    pub fn finish(mut self) -> [u8; 20] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0; 20];
        for (dst, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *dst = word.to_be_bytes();
        }
        out
    }
}

/// SHA-1 of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut hash = Sha1::new();
    hash.update(data);
    hash.finish()
}
