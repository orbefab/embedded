//! IEEE 802.11-2020 Annex J: the PRF test vectors of J.3, the pass-phrase-to-PSK test vectors of
//! J.4 and the CCMP test vector of J.6.4. hostap's `src/crypto/crypto_module_tests.c` carries the
//! same PRF and PSK values.

use orbe_crypto::aes::AesKey;
use orbe_crypto::ccm::{open, seal};
use orbe_crypto::pbkdf2::pbkdf2_sha1;
use orbe_crypto::prf::prf;

use super::decode;

/// J.3: PRF-512 with the prefix "prefix" over three keys and data from RFC 2202.
#[test]
fn prf_matches_annex_j_3() {
    let cases: [(&[u8], &[u8], &str); 3] = [
        (
            &[0x0b; 20],
            b"Hi There",
            "bcd4c650b30b9684951829e0d75f9d54b862175ed9f00606e17d8da35402ffee\
             75df78c3d31e0f889f012120c0862beb67753e7439ae242edb8373698356cf5a",
        ),
        (
            b"Jefe",
            b"what do ya want for nothing?",
            "51f4de5b33f249adf81aeb713a3c20f4fe631446fabdfa58244759ae58ef9009\
             a99abf4eac2ca5fa87e692c440eb40023e7babb206d61de7b92f41529092b8fc",
        ),
        (
            &[0xaa; 20],
            &[0xdd; 50],
            "e1ac546ec4cb636f9976487be5c86be17a0252ca5d8d8df12cfb0473525249ce\
             9dd8d177ead710bc9b590547239107aef7b4abd43d87f0a68f1cbd9e2b6f7607",
        ),
    ];
    for (i, (key, data, want)) in cases.into_iter().enumerate() {
        let mut out = [0; 64];
        prf(key, b"prefix", data, &mut out);
        assert_eq!(out.to_vec(), decode(want), "test case {}", i + 1);
        // A shorter output is a prefix of the longer one, as the PTK of 384 bits is.
        let mut short = [0; 48];
        prf(key, b"prefix", data, &mut short);
        assert_eq!(short, out[..48], "test case {} cut to 384 bits", i + 1);
    }
}

/// J.4: the PSK is PBKDF2-HMAC-SHA1 of the passphrase over the SSID, 4096 iterations, 256 bits.
#[test]
fn psk_matches_annex_j_4() {
    let cases: [(&[u8], &[u8], &str); 3] = [
        (
            b"password",
            b"IEEE",
            "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e",
        ),
        (
            b"ThisIsAPassword",
            b"ThisIsASSID",
            "0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af",
        ),
        (
            &[b'a'; 32],
            &[b'Z'; 32],
            "becb93866bb8c3832cb777c2f559807c8c59afcb6eae734885001300a981cc62",
        ),
    ];
    for (passphrase, ssid, want) in cases {
        let mut psk = [0; 32];
        pbkdf2_sha1(passphrase, ssid, 4096, &mut psk);
        assert_eq!(
            psk.to_vec(),
            decode(want),
            "{}",
            String::from_utf8_lossy(ssid)
        );
    }
}

/// J.6.4: one CCMP data MPDU. The annex gives the nonce and the AAD it builds from the header
/// and the PN, the plaintext, and the encrypted data and the 8-byte MIC.
#[test]
fn ccmp_matches_annex_j_6() {
    let tk = AesKey::new(&decode("c97c1f67ce371185514a8a19f2bdd52f")).unwrap();
    let nonce = decode("005030f1844408b5039776e70c");
    let aad = decode("08400fd2e128a57c5030f1844408abaea5b8fcba0000");
    let plaintext = decode("f8ba1a55d02f85ae967bb62fb6cda8eb7e78a050");
    let sealed = decode("f3d0a2fe9a3dbf2342a643e43246e80c3c04d0197845ce0b16f97623");
    assert_eq!(seal(&tk, &nonce, &aad, &plaintext, 8), sealed);
    assert_eq!(open(&tk, &nonce, &aad, &sealed, 8), Some(plaintext));
}
