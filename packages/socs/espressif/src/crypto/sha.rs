//! Chip-agnostic SHA compression functions and initial hash values per FIPS 180-4.
//!
//! Supports SHA-1 (160-bit), SHA-224, SHA-256, SHA-384, SHA-512, SHA-512/224, SHA-512/256 and
//! SHA-512/t. SHA-224/256 and SHA-384/512 delegate block compression to `sha2`'s `compress256`
//! and `compress512`; SHA-1 compression is implemented directly from FIPS 180-4 §6.1.

use sha2::digest::generic_array::GenericArray;
use sha2::{compress256, compress512};

/// Initial hash value for SHA-1 (FIPS 180-4 §5.3.1).
pub(crate) const SHA1_IV: [u32; 5] = [
    0x6745_2301,
    0xefcd_ab89,
    0x98ba_dcfe,
    0x1032_5476,
    0xc3d2_e1f0,
];

/// Initial hash value for SHA-224 (FIPS 180-4 §5.3.2).
pub(crate) const SHA224_IV: [u32; 8] = [
    0xc105_9ed8,
    0x367c_d507,
    0x3070_dd17,
    0xf70e_5939,
    0xffc0_0b31,
    0x6858_1511,
    0x64f9_8fa7,
    0xbefa_4fa4,
];

/// Initial hash value for SHA-256 (FIPS 180-4 §5.3.3).
pub(crate) const SHA256_IV: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// Initial hash value for SHA-384 (FIPS 180-4 §5.3.4).
pub(crate) const SHA384_IV: [u64; 8] = [
    0xcbbb_9d5d_c105_9ed8,
    0x629a_292a_367c_d507,
    0x9159_015a_3070_dd17,
    0x152f_ecd8_f70e_5939,
    0x6733_2667_ffc0_0b31,
    0x8eb4_4a87_6858_1511,
    0xdb0c_2e0d_64f9_8fa7,
    0x47b5_481d_befa_4fa4,
];

/// Initial hash value for SHA-512 (FIPS 180-4 §5.3.5).
pub(crate) const SHA512_IV: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

/// Initial hash value for SHA-512/224 (FIPS 180-4 §5.3.6.1).
pub(crate) const SHA512_224_IV: [u64; 8] = [
    0x8c3d_37c8_1954_4da2,
    0x73e1_9966_89dc_d4d6,
    0x1dfa_b7ae_32ff_9c82,
    0x679d_d514_582f_9fcf,
    0x0f6d_2b69_7bd4_4da8,
    0x77e3_6f73_04c4_8942,
    0x3f9d_85a8_6a1d_36c8,
    0x1112_e6ad_91d6_92a1,
];

/// Initial hash value for SHA-512/256 (FIPS 180-4 §5.3.6.2).
pub(crate) const SHA512_256_IV: [u64; 8] = [
    0x2231_2194_fc2b_f72c,
    0x9f55_5fa3_c84c_64c2,
    0x2393_b86b_6f53_b151,
    0x9638_7719_5940_eabd,
    0x9628_3ee2_a88e_ffe3,
    0xbe5e_1e25_5386_3992,
    0x2b01_99fc_2c85_b8aa,
    0x0eb7_2ddc_81c5_2ca2,
];

/// Compresses one or more 64-byte blocks with SHA-1 (FIPS 180-4 §6.1.2).
pub(crate) fn compress1(state: &mut [u32; 5], bytes: &[u8]) {
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

/// Compresses one or more 64-byte blocks with the SHA-256 core (SHA-224 and SHA-256).
pub(crate) fn compress_256(state: &mut [u32; 8], bytes: &[u8]) {
    let (chunks, remainder) = bytes.as_chunks::<64>();
    debug_assert!(remainder.is_empty());
    let blocks: Vec<_> = chunks
        .iter()
        .map(|block| GenericArray::from(*block))
        .collect();
    compress256(state, &blocks);
}

/// Compresses one or more 128-byte blocks with the SHA-512 core (SHA-384, SHA-512 and SHA-512/t).
pub(crate) fn compress_512(state: &mut [u64; 8], bytes: &[u8]) {
    let (chunks, remainder) = bytes.as_chunks::<128>();
    debug_assert!(remainder.is_empty());
    let blocks: Vec<_> = chunks
        .iter()
        .map(|block| GenericArray::from(*block))
        .collect();
    compress512(state, &blocks);
}

/// Computes the initial hash value for SHA-512/t from the ESP32-S3's `t_string` (ASCII digits of
/// `t` followed by the `0x80` padding byte, big-endian in a 32-bit word) and `t_len` (total bit
/// length of `"SHA-512/" || digits`).
///
/// Matches FIPS 180-4 §5.3.6: start from `SHA512_IV ^ 0xa5a5...a5` and compress the single
/// 128-byte block `"SHA-512/" || t_string || 0...0 || t_len_be128`.
pub(crate) fn sha512_t_iv(t_string: u32, t_len: u32) -> [u64; 8] {
    let mut state = SHA512_IV.map(|word| word ^ 0xa5a5_a5a5_a5a5_a5a5);
    let mut block = [0u8; 128];
    block[..8].copy_from_slice(b"SHA-512/");
    block[8..12].copy_from_slice(&t_string.to_be_bytes());
    block[120..128].copy_from_slice(&u64::from(t_len).to_be_bytes());
    compress_512(&mut state, &block);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn pad64(msg: &[u8]) -> Vec<u8> {
        let mut out = msg.to_vec();
        out.push(0x80);
        while out.len() % 64 != 56 {
            out.push(0);
        }
        out.extend_from_slice(&(8 * msg.len() as u64).to_be_bytes());
        out
    }

    fn pad128(msg: &[u8]) -> Vec<u8> {
        let mut out = msg.to_vec();
        out.push(0x80);
        while out.len() % 128 != 112 {
            out.push(0);
        }
        out.extend_from_slice(&(8 * msg.len() as u128).to_be_bytes());
        out
    }

    fn sha1(msg: &[u8]) -> Vec<u8> {
        let mut s = SHA1_IV;
        compress1(&mut s, &pad64(msg));
        s.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn sha224(msg: &[u8]) -> Vec<u8> {
        let mut s = SHA224_IV;
        compress_256(&mut s, &pad64(msg));
        s.iter().flat_map(|w| w.to_be_bytes()).take(28).collect()
    }

    fn sha256(msg: &[u8]) -> Vec<u8> {
        let mut s = SHA256_IV;
        compress_256(&mut s, &pad64(msg));
        s.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn sha384(msg: &[u8]) -> Vec<u8> {
        let mut s = SHA384_IV;
        compress_512(&mut s, &pad128(msg));
        s.iter().flat_map(|w| w.to_be_bytes()).take(48).collect()
    }

    fn sha512(msg: &[u8]) -> Vec<u8> {
        let mut s = SHA512_IV;
        compress_512(&mut s, &pad128(msg));
        s.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// Official FIPS 180-4 / NIST CAVP test vectors for SHA-1.
    #[test]
    fn sha1_matches_fips_180_4_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    /// Official FIPS 180-4 test vectors for SHA-224 and SHA-256.
    #[test]
    fn sha224_and_sha256_match_fips_180_4_vectors() {
        assert_eq!(
            hex(&sha224(b"abc")),
            "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7"
        );
        assert_eq!(
            hex(&sha224(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "75388b16512776cc5dba5da1fd890150b0c6455cb4f58b1952522525"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// Official FIPS 180-4 test vectors for SHA-384, SHA-512, SHA-512/224 and SHA-512/256.
    #[test]
    fn sha384_sha512_and_sha512_t_match_fips_180_4_vectors() {
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
             8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        // Verify SHA-512/224 and SHA-512/256 IV generation via sha512_t_iv.
        let iv224 = sha512_t_iv(0x3232_3480, 0x58);
        assert_eq!(iv224, SHA512_224_IV);
        let iv256 = sha512_t_iv(0x3235_3680, 0x58);
        assert_eq!(iv256, SHA512_256_IV);

        let mut s = SHA512_224_IV;
        compress_512(&mut s, &pad128(b"abc"));
        let d224: Vec<u8> = s.iter().flat_map(|w| w.to_be_bytes()).take(28).collect();
        assert_eq!(
            hex(&d224),
            "4634270f707b6a54daae7530460842e20e37ed265ceee9a43e8924aa"
        );

        let mut s = SHA512_256_IV;
        compress_512(&mut s, &pad128(b"abc"));
        let d256: Vec<u8> = s.iter().flat_map(|w| w.to_be_bytes()).take(32).collect();
        assert_eq!(
            hex(&d256),
            "53048e2681941ef99b2e29b76b4c7dabe4c2d0c634fc6d46e0e2f13107e7af23"
        );
    }
}
