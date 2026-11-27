//! AES key wrap (RFC 3394 §2.2), which carries the GTK in the third message of the WPA2 4-way
//! handshake.

use crate::aes::AesKey;

/// The default initial value, RFC 3394 §2.2.3.1.
const IV: [u8; 8] = [0xa6; 8];

/// Wraps `plaintext` under `kek`: 8 bytes longer. `None` unless `plaintext` is two or more
/// 64-bit blocks.
pub fn wrap(kek: &AesKey, plaintext: &[u8]) -> Option<Vec<u8>> {
    let n = blocks(plaintext)?;
    let mut out = Vec::with_capacity(plaintext.len() + 8);
    out.extend_from_slice(&IV);
    out.extend_from_slice(plaintext);
    let mut a = IV;
    for j in 0..6 {
        for i in 1..=n {
            let r = &mut out[8 * i..8 * i + 8];
            let b = kek.encrypt_block(&join(&a, r));
            a = b[..8].try_into().expect("8 bytes");
            xor_step(&mut a, n * j + i);
            r.copy_from_slice(&b[8..]);
        }
    }
    out[..8].copy_from_slice(&a);
    Some(out)
}

/// Unwraps `ciphertext` under `kek`. `None` unless it is three or more 64-bit blocks and its
/// integrity check holds.
pub fn unwrap(kek: &AesKey, ciphertext: &[u8]) -> Option<Vec<u8>> {
    let n = blocks(ciphertext.get(8..)?)?;
    let mut a: [u8; 8] = ciphertext[..8].try_into().expect("8 bytes");
    let mut out = ciphertext[8..].to_vec();
    for j in (0..6).rev() {
        for i in (1..=n).rev() {
            let r = &mut out[8 * (i - 1)..8 * i];
            xor_step(&mut a, n * j + i);
            let b = kek.decrypt_block(&join(&a, r));
            a = b[..8].try_into().expect("8 bytes");
            r.copy_from_slice(&b[8..]);
        }
    }
    (a == IV).then_some(out)
}

/// The number of 64-bit blocks in `data`, when it is a whole number of them and at least two.
fn blocks(data: &[u8]) -> Option<usize> {
    (data.len().is_multiple_of(8) && data.len() >= 16).then_some(data.len() / 8)
}

fn join(a: &[u8; 8], r: &[u8]) -> [u8; 16] {
    let mut block = [0; 16];
    block[..8].copy_from_slice(a);
    block[8..].copy_from_slice(r);
    block
}

fn xor_step(a: &mut [u8; 8], t: usize) {
    *a = (u64::from_be_bytes(*a) ^ t as u64).to_be_bytes();
}
