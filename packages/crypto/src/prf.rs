//! The IEEE 802.11 PRF (IEEE 802.11-2020 §12.7.1.2), which derives the PTK from the PMK, the two
//! addresses and the two nonces.

use crate::hmac::HmacSha1;

/// Fills `out` with `PRF(key, label, data, 8 * out.len())`: the HMAC-SHA1 of
/// `label || 0 || data || i` for `i` from 0, concatenated and cut to length.
///
/// # Panics
///
/// When `out` is longer than 255 HMAC-SHA1 blocks, 5100 bytes.
pub fn prf(key: &[u8], label: &[u8], data: &[u8], out: &mut [u8]) {
    assert!(out.len() <= 255 * 20, "the PRF counter is one byte");
    let keyed = HmacSha1::new(key);
    for (i, chunk) in (0u8..).zip(out.chunks_mut(20)) {
        let mut mac = keyed.clone();
        mac.update(label);
        mac.update(&[0]);
        mac.update(data);
        mac.update(&[i]);
        chunk.copy_from_slice(&mac.finish()[..chunk.len()]);
    }
}
