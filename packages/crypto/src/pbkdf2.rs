//! PBKDF2 with HMAC-SHA1 (RFC 8018 §5.2), which WPA2-PSK runs over the passphrase and the SSID
//! with 4096 iterations to make the 256-bit PSK.

use crate::hmac::HmacSha1;

/// Fills `out` with the key PBKDF2-HMAC-SHA1 derives from `password` and `salt`.
///
/// # Panics
///
/// When `iterations` is 0.
pub fn pbkdf2_sha1(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    assert!(iterations > 0, "PBKDF2 takes at least one iteration");
    let keyed = HmacSha1::new(password);
    for (index, chunk) in (1u32..).zip(out.chunks_mut(20)) {
        let mut mac = keyed.clone();
        mac.update(salt);
        mac.update(&index.to_be_bytes());
        let mut u = mac.finish();
        let mut t = u;
        for _ in 1..iterations {
            let mut mac = keyed.clone();
            mac.update(&u);
            u = mac.finish();
            for (t, u) in t.iter_mut().zip(u) {
                *t ^= u;
            }
        }
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
}
