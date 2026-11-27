//! FIPS 197 Appendix C (AES), NIST SP 800-38A Appendix F (block modes) and FIPS 180-4 with the
//! NIST CAVP examples (SHA-1).

use orbe_crypto::aes::{AesKey, BlockMode};
use orbe_crypto::sha1::{Sha1, sha1};

use super::{decode, decode16};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Official FIPS 197 Appendix C test vectors for AES-128, AES-192 and AES-256.
#[test]
fn fips_197_appendix_c_vectors() {
    let pt = decode16("00112233445566778899aabbccddeeff");
    let cases = [
        (
            "000102030405060708090a0b0c0d0e0f",
            "69c4e0d86a7b0430d8cdb78070b4c55a",
        ),
        (
            "000102030405060708090a0b0c0d0e0f1011121314151617",
            "dda97ca4864cdfe06eaf70a0ec0d7191",
        ),
        (
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8ea2b7ca516745bfeafc49904b496089",
        ),
    ];
    for (key_hex, ct_hex) in cases {
        let key = AesKey::new(&decode(key_hex)).unwrap();
        let ct = decode16(ct_hex);
        assert_eq!(key.encrypt_block(&pt), ct, "encrypt {key_hex}");
        assert_eq!(key.decrypt_block(&ct), pt, "decrypt {key_hex}");
    }
}

/// Official NIST SP 800-38A Appendix F test vectors for ECB, CBC, CFB128, CFB8, OFB and CTR.
#[test]
fn sp_800_38a_block_modes() {
    let key128 = AesKey::new(&decode("2b7e151628aed2a6abf7158809cf4f3c")).unwrap();
    let key256 = AesKey::new(&decode(
        "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
    ))
    .unwrap();
    let pt = decode(
        "6bc1bee22e409f96e93d7e117393172a\
         ae2d8a571e03ac9c9eb76fac45af8e51\
         30c81c46a35ce411e5fbc1191a0a52ef\
         f69f2445df4f9b17ad2b417be66c3710",
    );
    let iv0 = decode16("000102030405060708090a0b0c0d0e0f");

    // F.1.1 ECB-AES128
    let ecb128 = decode(
        "3ad77bb40d7a3660a89ecaf32466ef97\
         f5d3d58503b9699de785895a96fdbaaf\
         43b1cd7f598ece23881b00e3ed030688\
         7b0c785e27e8ad3f8223207104725dd4",
    );
    let mut iv = [0; 16];
    assert_eq!(key128.transform(BlockMode::Ecb, true, &mut iv, &pt), ecb128);
    assert_eq!(
        key128.transform(BlockMode::Ecb, false, &mut iv, &ecb128),
        pt
    );

    // F.1.5 ECB-AES256
    let ecb256 = decode(
        "f3eed1bdb5d2a03c064b5a7e3db181f8\
         591ccb10d410ed26dc5ba74a31362870\
         b6ed21b99ca6f4f9f153e7b1beafed1d\
         23304b7a39f9f3ff067d8d8f9e24ecc7",
    );
    assert_eq!(key256.transform(BlockMode::Ecb, true, &mut iv, &pt), ecb256);
    assert_eq!(
        key256.transform(BlockMode::Ecb, false, &mut iv, &ecb256),
        pt
    );

    // F.2.1 CBC-AES128
    let cbc128 = decode(
        "7649abac8119b246cee98e9b12e9197d\
         5086cb9b507219ee95db113a917678b2\
         73bed6b8e3c1743b7116e69e22229516\
         3ff1caa1681fac09120eca307586e1a7",
    );
    let mut iv = iv0;
    assert_eq!(key128.transform(BlockMode::Cbc, true, &mut iv, &pt), cbc128);
    assert_eq!(iv, cbc128[48..64]);
    let mut iv = iv0;
    assert_eq!(
        key128.transform(BlockMode::Cbc, false, &mut iv, &cbc128),
        pt
    );
    assert_eq!(iv, cbc128[48..64]);

    // F.2.5 CBC-AES256
    let cbc256 = decode(
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6\
         9cfc4e967edb808d679f777bc6702c7d\
         39f23369a9d9bacfa530e26304231461\
         b2eb05e2c39be9fcda6c19078c6a9d1b",
    );
    let mut iv = iv0;
    assert_eq!(key256.transform(BlockMode::Cbc, true, &mut iv, &pt), cbc256);
    let mut iv = iv0;
    assert_eq!(
        key256.transform(BlockMode::Cbc, false, &mut iv, &cbc256),
        pt
    );

    // F.3.13 CFB128-AES128
    let cfb128 = decode(
        "3b3fd92eb72dad20333449f8e83cfb4a\
         c8a64537a0b3a93fcde3cdad9f1ce58b\
         26751f67a3cbb140b1808cf187a4f4df\
         c04b05357c5d1c0eeac4c66f9ff7f2e6",
    );
    let mut iv = iv0;
    assert_eq!(
        key128.transform(BlockMode::Cfb128, true, &mut iv, &pt),
        cfb128
    );
    let mut iv = iv0;
    assert_eq!(
        key128.transform(BlockMode::Cfb128, false, &mut iv, &cfb128),
        pt
    );

    // F.3.7 CFB8-AES128 (first 16 bytes; SP 800-38A lists the first 18 bytes)
    let cfb8_pt = &pt[..16];
    let cfb8_ct = decode("3b79424c9c0dd436bace9e0ed4586a4f");
    let mut iv = iv0;
    assert_eq!(
        key128.transform(BlockMode::Cfb8, true, &mut iv, cfb8_pt),
        cfb8_ct
    );
    let mut iv = iv0;
    assert_eq!(
        key128.transform(BlockMode::Cfb8, false, &mut iv, &cfb8_ct),
        cfb8_pt
    );

    // F.4.1 OFB-AES128
    let ofb128 = decode(
        "3b3fd92eb72dad20333449f8e83cfb4a\
         7789508d16918f03f53c52dac54ed825\
         9740051e9c5fecf64344f7a82260edcc\
         304c6528f659c77866a510d9c1d6ae5e",
    );
    let mut iv = iv0;
    assert_eq!(key128.transform(BlockMode::Ofb, true, &mut iv, &pt), ofb128);
    let mut iv = iv0;
    assert_eq!(
        key128.transform(BlockMode::Ofb, false, &mut iv, &ofb128),
        pt
    );

    // F.5.1 CTR-AES128
    let ctr_iv = decode16("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
    let ctr128 = decode(
        "874d6191b620e3261bef6864990db6ce\
         9806f66b7970fdff8617187bb9fffdff\
         5ae4df3edbd5d35e5b4f09020db03eab\
         1e031dda2fbe03d1792170a0f3009cee",
    );
    let mut iv = ctr_iv;
    assert_eq!(
        key128.transform(BlockMode::Ctr { inc128: false }, true, &mut iv, &pt),
        ctr128
    );
    assert_eq!(iv, decode16("f0f1f2f3f4f5f6f7f8f9fafbfcfdff03"));
    let mut iv = ctr_iv;
    assert_eq!(
        key128.transform(BlockMode::Ctr { inc128: false }, false, &mut iv, &ctr128),
        pt
    );

    // Verify INC32 wraps only the low 32 bits while INC128 carries into the upper 96 bits.
    let wrap_iv = decode16("000000000000000000000000ffffffff");
    let mut iv32 = wrap_iv;
    key128.transform(BlockMode::Ctr { inc128: false }, true, &mut iv32, &[0; 16]);
    assert_eq!(iv32, decode16("00000000000000000000000000000000"));
    let mut iv128 = wrap_iv;
    key128.transform(BlockMode::Ctr { inc128: true }, true, &mut iv128, &[0; 16]);
    assert_eq!(iv128, decode16("00000000000000000000000100000000"));
}

/// FIPS 180-4 / NIST CAVP SHA-1 examples: the empty message, "abc" and the two-block message.
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

/// The digest does not depend on how the message is split.
#[test]
fn sha1_is_the_same_over_any_split() {
    let message: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
    let whole = sha1(&message);
    for a in [0, 1, 55, 56, 63, 64, 65, 128, 299] {
        for b in [a, a + 1, a + 64, 300] {
            let b = b.min(300);
            let mut hash = Sha1::new();
            hash.update(&message[..a]);
            hash.update(&message[a..b]);
            hash.update(&message[b..]);
            assert_eq!(hash.finish(), whole, "split at {a} and {b}");
        }
    }
}
