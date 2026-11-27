//! RFC 6070 §2, the PBKDF2-HMAC-SHA1 test vectors, except the one of 16777216 iterations, which
//! takes seconds.

use orbe_crypto::pbkdf2::pbkdf2_sha1;

use super::decode;

#[test]
fn pbkdf2_sha1_matches_rfc_6070() {
    let cases: [(&[u8], &[u8], u32, &str); 5] = [
        (
            b"password",
            b"salt",
            1,
            "0c60c80f961f0e71f3a9b524af6012062fe037a6",
        ),
        (
            b"password",
            b"salt",
            2,
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957",
        ),
        (
            b"password",
            b"salt",
            4096,
            "4b007901b765489abead49d926f721d065a429c1",
        ),
        (
            b"passwordPASSWORDpassword",
            b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
            4096,
            "3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038",
        ),
        (
            b"pass\0word",
            b"sa\0lt",
            4096,
            "56fa6aa75548099dcc37d7f03425e0c3",
        ),
    ];
    for (password, salt, iterations, dk) in cases {
        let want = decode(dk);
        let mut got = vec![0; want.len()];
        pbkdf2_sha1(password, salt, iterations, &mut got);
        assert_eq!(got, want, "{} iterations, {} bytes", iterations, want.len());
    }
}
