//! RFC 3394 §4, the AES key wrap test vectors.

use orbe_crypto::aes::AesKey;
use orbe_crypto::key_wrap::{unwrap, wrap};

use super::decode;

const KEK: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const KEY_DATA: &str = "00112233445566778899aabbccddeeff000102030405060708090a0b0c0d0e0f";

#[test]
fn key_wrap_matches_rfc_3394() {
    // (section, KEK bytes, key data bytes, ciphertext)
    let cases = [
        (
            "4.1",
            16,
            16,
            "1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5",
        ),
        (
            "4.2",
            24,
            16,
            "96778b25ae6ca435f92b5b97c050aed2468ab8a17ad84e5d",
        ),
        (
            "4.3",
            32,
            16,
            "64e8c3f9ce0f5ba263e9777905818a2a93c8191e7d6e8ae7",
        ),
        (
            "4.4",
            24,
            24,
            "031d33264e15d33268f24ec260743edce1c6c7ddee725a936ba814915c6762d2",
        ),
        (
            "4.5",
            32,
            24,
            "a8f9bc1612c68b3ff6e6f4fbe30e71e4769c8b80a32cb8958cd5d17d6b254da1",
        ),
        (
            "4.6",
            32,
            32,
            "28c9f404c4b810f4cbccb35cfb87f8263f5786e2d80ed326cbc7f0e71a99f43bfb988b9b7a02dd21",
        ),
    ];
    for (section, kek_len, data_len, ciphertext) in cases {
        let kek = AesKey::new(&decode(KEK)[..kek_len]).unwrap();
        let data = &decode(KEY_DATA)[..data_len];
        let ciphertext = decode(ciphertext);
        assert_eq!(
            wrap(&kek, data).as_deref(),
            Some(&ciphertext[..]),
            "wrap {section}"
        );
        assert_eq!(
            unwrap(&kek, &ciphertext).as_deref(),
            Some(data),
            "unwrap {section}"
        );
    }
}

#[test]
fn key_wrap_refuses_a_bad_length_or_a_changed_byte() {
    let kek = AesKey::new(&decode(KEK)[..16]).unwrap();
    let mut ciphertext = decode("1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5");
    assert_eq!(wrap(&kek, &[0; 8]), None);
    assert_eq!(wrap(&kek, &[0; 20]), None);
    assert_eq!(unwrap(&kek, &ciphertext[..16]), None);
    assert_eq!(unwrap(&kek, &ciphertext[..23]), None);
    ciphertext[12] ^= 1;
    assert_eq!(unwrap(&kek, &ciphertext), None);
}
