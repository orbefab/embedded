//! RFC 2202 §3, the HMAC-SHA1 test cases.

use orbe_crypto::hmac::hmac_sha1;

use super::decode;

#[test]
fn hmac_sha1_matches_rfc_2202() {
    let long_key = [0xaa; 80];
    let cases: [(&[u8], &[u8], &str); 7] = [
        (
            &[0x0b; 20],
            b"Hi There",
            "b617318655057264e28bc0b6fb378c8ef146be00",
        ),
        (
            b"Jefe",
            b"what do ya want for nothing?",
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79",
        ),
        (
            &[0xaa; 20],
            &[0xdd; 50],
            "125d7342b9ac11cd91a39af48aa17b4f63f175d3",
        ),
        (
            &decode("0102030405060708090a0b0c0d0e0f10111213141516171819"),
            &[0xcd; 50],
            "4c9007f4026250c6bc8414f9bf50c86c2d7235da",
        ),
        (
            &[0x0c; 20],
            b"Test With Truncation",
            "4c1a03424b55e07fe7f27be1d58bb9324a9a5a04",
        ),
        (
            &long_key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
            "aa4ae5e15272d00e95705637ce8a3b55ed402112",
        ),
        (
            &long_key,
            b"Test Using Larger Than Block-Size Key and Larger Than One Block-Size Data",
            "e8e99d0f45237d786d6bbaa7965c7808bbff1a91",
        ),
    ];
    for (i, (key, data, digest)) in cases.into_iter().enumerate() {
        assert_eq!(
            hmac_sha1(key, data).to_vec(),
            decode(digest),
            "test case {}",
            i + 1
        );
    }
}
