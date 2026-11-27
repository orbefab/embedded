//! The AES block cipher (FIPS 197) and its block modes (NIST SP 800-38A).
//!
//! AES-128, AES-192 (for the classic ESP32's accelerator) and AES-256, plus ECB, CBC, OFB, CTR
//! (32-bit and 128-bit counter increment), CFB8 and CFB128.

/// Forward AES S-box (FIPS 197 §5.1.1).
const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

/// Inverse AES S-box (FIPS 197 §5.3.2).
const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

/// Round constants `Rcon[1..=10]` (FIPS 197 §5.2).
const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

/// Block cipher mode of operation (NIST SP 800-38A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockMode {
    Ecb,
    Cbc,
    Ofb,
    Ctr { inc128: bool },
    Cfb8,
    Cfb128,
}

/// Expanded AES key schedule for 128-, 192- or 256-bit keys.
#[derive(Clone, Debug)]
pub struct AesKey {
    /// Round keys, `rounds + 1` blocks of 16 bytes (max 15 for AES-256).
    round_keys: [[u8; 16]; 15],
    rounds: usize,
}

impl AesKey {
    /// Expands a 16-, 24- or 32-byte key into its round keys per FIPS 197 §5.2.
    pub fn new(key: &[u8]) -> Option<AesKey> {
        let (nk, rounds) = match key.len() {
            16 => (4, 10),
            24 => (6, 12),
            32 => (8, 14),
            _ => return None,
        };
        let total_words = 4 * (rounds + 1);
        let mut w = [[0u8; 4]; 60];
        for i in 0..nk {
            w[i].copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        for i in nk..total_words {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = [
                    SBOX[usize::from(temp[1])] ^ RCON[i / nk - 1],
                    SBOX[usize::from(temp[2])],
                    SBOX[usize::from(temp[3])],
                    SBOX[usize::from(temp[0])],
                ];
            } else if nk > 6 && i % nk == 4 {
                temp = temp.map(|b| SBOX[usize::from(b)]);
            }
            for k in 0..4 {
                w[i][k] = w[i - nk][k] ^ temp[k];
            }
        }
        let mut round_keys = [[0u8; 16]; 15];
        for (r, rk) in round_keys[..=rounds].iter_mut().enumerate() {
            for c in 0..4 {
                rk[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        Some(AesKey { round_keys, rounds })
    }

    /// Encrypts one 16-byte block per FIPS 197 §5.1.
    pub fn encrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut s = *input;
        add_round_key(&mut s, &self.round_keys[0]);
        for r in 1..self.rounds {
            sub_bytes(&mut s);
            shift_rows(&mut s);
            mix_columns(&mut s);
            add_round_key(&mut s, &self.round_keys[r]);
        }
        sub_bytes(&mut s);
        shift_rows(&mut s);
        add_round_key(&mut s, &self.round_keys[self.rounds]);
        s
    }

    /// Decrypts one 16-byte block per FIPS 197 §5.3.
    pub fn decrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let mut s = *input;
        add_round_key(&mut s, &self.round_keys[self.rounds]);
        for r in (1..self.rounds).rev() {
            inv_shift_rows(&mut s);
            inv_sub_bytes(&mut s);
            add_round_key(&mut s, &self.round_keys[r]);
            inv_mix_columns(&mut s);
        }
        inv_shift_rows(&mut s);
        inv_sub_bytes(&mut s);
        add_round_key(&mut s, &self.round_keys[0]);
        s
    }

    /// Transforms `input` in `mode`, updating `iv` in place.
    ///
    /// `encrypt` selects encryption vs decryption for ECB, CBC, CFB8 and CFB128; OFB and CTR
    /// always apply the forward cipher regardless of `encrypt`.
    ///
    /// # Panics
    ///
    /// When `input` is not a whole number of 16-byte blocks in any mode other than
    /// [`BlockMode::Cfb8`].
    pub fn transform(
        &self,
        mode: BlockMode,
        encrypt: bool,
        iv: &mut [u8; 16],
        input: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        let whole_blocks = || {
            let (blocks, remainder) = input.as_chunks::<16>();
            assert!(
                remainder.is_empty(),
                "AES {mode:?} expects whole 16-byte blocks, got {} bytes",
                input.len()
            );
            blocks
        };
        match mode {
            BlockMode::Ecb => {
                for block in whole_blocks() {
                    let result = if encrypt {
                        self.encrypt_block(block)
                    } else {
                        self.decrypt_block(block)
                    };
                    out.extend_from_slice(&result);
                }
            }
            BlockMode::Cbc => {
                for block in whole_blocks() {
                    if encrypt {
                        let xored = xor_block(block, iv);
                        let cipher = self.encrypt_block(&xored);
                        *iv = cipher;
                        out.extend_from_slice(&cipher);
                    } else {
                        let plain = xor_block(&self.decrypt_block(block), iv);
                        *iv = *block;
                        out.extend_from_slice(&plain);
                    }
                }
            }
            BlockMode::Ofb => {
                for block in whole_blocks() {
                    let stream = self.encrypt_block(iv);
                    *iv = stream;
                    out.extend_from_slice(&xor_block(block, &stream));
                }
            }
            BlockMode::Ctr { inc128 } => {
                for block in whole_blocks() {
                    let stream = self.encrypt_block(iv);
                    out.extend_from_slice(&xor_block(block, &stream));
                    increment_counter(iv, inc128);
                }
            }
            BlockMode::Cfb8 => {
                for &byte in input {
                    let stream = self.encrypt_block(iv);
                    let produced = byte ^ stream[0];
                    iv.copy_within(1..16, 0);
                    iv[15] = if encrypt { produced } else { byte };
                    out.push(produced);
                }
            }
            BlockMode::Cfb128 => {
                for block in whole_blocks() {
                    let stream = self.encrypt_block(iv);
                    let produced = xor_block(block, &stream);
                    *iv = if encrypt { produced } else { *block };
                    out.extend_from_slice(&produced);
                }
            }
        }
        out
    }
}

fn xor_block(a: &[u8; 16], b: &[u8; 16]) -> [u8; 16] {
    std::array::from_fn(|i| a[i] ^ b[i])
}

fn increment_counter(counter: &mut [u8; 16], inc128: bool) {
    if inc128 {
        let next = u128::from_be_bytes(*counter).wrapping_add(1);
        *counter = next.to_be_bytes();
    } else {
        let low = u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]])
            .wrapping_add(1);
        counter[12..16].copy_from_slice(&low.to_be_bytes());
    }
}

fn add_round_key(state: &mut [u8; 16], key: &[u8; 16]) {
    for (s, &k) in state.iter_mut().zip(key) {
        *s ^= k;
    }
}

fn sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = SBOX[usize::from(*b)];
    }
}

fn inv_sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = INV_SBOX[usize::from(*b)];
    }
}

fn shift_rows(s: &mut [u8; 16]) {
    // Row 1: shift left by 1
    let t = s[1];
    s[1] = s[5];
    s[5] = s[9];
    s[9] = s[13];
    s[13] = t;
    // Row 2: shift left by 2
    s.swap(2, 10);
    s.swap(6, 14);
    // Row 3: shift left by 3 (right by 1)
    let t = s[15];
    s[15] = s[11];
    s[11] = s[7];
    s[7] = s[3];
    s[3] = t;
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    // Row 1: shift right by 1
    let t = s[13];
    s[13] = s[9];
    s[9] = s[5];
    s[5] = s[1];
    s[1] = t;
    // Row 2: shift right by 2
    s.swap(2, 10);
    s.swap(6, 14);
    // Row 3: shift right by 3 (left by 1)
    let t = s[3];
    s[3] = s[7];
    s[7] = s[11];
    s[11] = s[15];
    s[15] = t;
}

#[inline]
fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 }
}

#[inline]
fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

fn mix_columns(s: &mut [u8; 16]) {
    for col in s.as_chunks_mut::<4>().0 {
        let [a0, a1, a2, a3] = *col;
        let t = a0 ^ a1 ^ a2 ^ a3;
        col[0] ^= t ^ xtime(a0 ^ a1);
        col[1] ^= t ^ xtime(a1 ^ a2);
        col[2] ^= t ^ xtime(a2 ^ a3);
        col[3] ^= t ^ xtime(a3 ^ a0);
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for col in s.as_chunks_mut::<4>().0 {
        let [a0, a1, a2, a3] = *col;
        col[0] = gf_mul(a0, 0x0e) ^ gf_mul(a1, 0x0b) ^ gf_mul(a2, 0x0d) ^ gf_mul(a3, 0x09);
        col[1] = gf_mul(a0, 0x09) ^ gf_mul(a1, 0x0e) ^ gf_mul(a2, 0x0b) ^ gf_mul(a3, 0x0d);
        col[2] = gf_mul(a0, 0x0d) ^ gf_mul(a1, 0x09) ^ gf_mul(a2, 0x0e) ^ gf_mul(a3, 0x0b);
        col[3] = gf_mul(a0, 0x0b) ^ gf_mul(a1, 0x0d) ^ gf_mul(a2, 0x09) ^ gf_mul(a3, 0x0e);
    }
}
