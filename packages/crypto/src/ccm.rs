//! CCM, counter mode with CBC-MAC (RFC 3610), over AES. CCMP is CCM with a 13-byte nonce and an
//! 8-byte MIC (IEEE 802.11-2020 §12.5.3).

use crate::aes::AesKey;

/// Encrypts `plaintext` under `key` and authenticates it with `aad`: the ciphertext, then a
/// `tag_len`-byte tag.
///
/// # Panics
///
/// When `nonce` is not 7 to 13 bytes, `tag_len` is not an even number from 4 to 16, or
/// `plaintext` does not fit the length field the nonce leaves.
pub fn seal(key: &AesKey, nonce: &[u8], aad: &[u8], plaintext: &[u8], tag_len: usize) -> Vec<u8> {
    let params = Params::new(nonce, tag_len);
    assert!(
        params.fits(plaintext.len()),
        "CCM message too long for its nonce"
    );
    let tag = params.tag(key, aad, plaintext);
    let mut out = params.ctr(key, plaintext);
    out.extend_from_slice(&tag[..tag_len]);
    out
}

/// Decrypts and checks what [`seal`] made. `None` when `sealed` is shorter than its tag, too long
/// for the nonce, or fails its check.
///
/// # Panics
///
/// When `nonce` or `tag_len` is out of range, as for [`seal`].
pub fn open(
    key: &AesKey,
    nonce: &[u8],
    aad: &[u8],
    sealed: &[u8],
    tag_len: usize,
) -> Option<Vec<u8>> {
    let params = Params::new(nonce, tag_len);
    let body = sealed.len().checked_sub(tag_len)?;
    if !params.fits(body) {
        return None;
    }
    let plaintext = params.ctr(key, &sealed[..body]);
    let tag = params.tag(key, aad, &plaintext);
    (tag[..tag_len] == sealed[body..]).then_some(plaintext)
}

struct Params<'a> {
    nonce: &'a [u8],
    tag_len: usize,
    /// The length field's width in bytes, L in RFC 3610 §2.
    width: usize,
}

impl Params<'_> {
    fn new(nonce: &[u8], tag_len: usize) -> Params<'_> {
        assert!(
            (7..=13).contains(&nonce.len()),
            "CCM nonce of {} bytes",
            nonce.len()
        );
        assert!(
            (4..=16).contains(&tag_len) && tag_len.is_multiple_of(2),
            "CCM tag of {tag_len} bytes"
        );
        Params {
            nonce,
            tag_len,
            width: 15 - nonce.len(),
        }
    }

    fn fits(&self, len: usize) -> bool {
        self.width == 8 || (len as u64) >> (8 * self.width) == 0
    }

    /// A block of flags, the nonce and `value` in the length field.
    fn block(&self, flags: u8, value: usize) -> [u8; 16] {
        let mut block = [0; 16];
        block[0] = flags;
        block[1..1 + self.nonce.len()].copy_from_slice(self.nonce);
        let value = (value as u64).to_be_bytes();
        block[16 - self.width..].copy_from_slice(&value[8 - self.width..]);
        block
    }

    /// The unencrypted tag, the CBC-MAC of RFC 3610 §2.2.
    fn tag(&self, key: &AesKey, aad: &[u8], message: &[u8]) -> [u8; 16] {
        let flags = (u8::from(!aad.is_empty()) << 6)
            | (((self.tag_len - 2) / 2) as u8) << 3
            | (self.width - 1) as u8;
        let mut x = key.encrypt_block(&self.block(flags, message.len()));
        if !aad.is_empty() {
            let mut encoded = Vec::with_capacity(aad.len() + 6);
            if aad.len() < 0xff00 {
                encoded.extend_from_slice(&(aad.len() as u16).to_be_bytes());
            } else {
                encoded.extend_from_slice(&[0xff, 0xfe]);
                let len = u32::try_from(aad.len()).expect("CCM associated data under 4 GiB");
                encoded.extend_from_slice(&len.to_be_bytes());
            }
            encoded.extend_from_slice(aad);
            x = mac(key, x, &encoded);
        }
        let x = mac(key, x, message);
        // S_0 masks the tag (RFC 3610 §2.3).
        let s0 = key.encrypt_block(&self.block((self.width - 1) as u8, 0));
        std::array::from_fn(|i| x[i] ^ s0[i])
    }

    /// Counter mode from A_1 (RFC 3610 §2.3).
    fn ctr(&self, key: &AesKey, data: &[u8]) -> Vec<u8> {
        let flags = (self.width - 1) as u8;
        data.chunks(16)
            .zip(1..)
            .flat_map(|(chunk, i)| {
                let s = key.encrypt_block(&self.block(flags, i));
                chunk.iter().zip(s).map(|(b, s)| b ^ s).collect::<Vec<_>>()
            })
            .collect()
    }
}

/// CBC-MAC over `data`, zero-padded to whole blocks, from `x`.
fn mac(key: &AesKey, mut x: [u8; 16], data: &[u8]) -> [u8; 16] {
    for chunk in data.chunks(16) {
        for (x, b) in x.iter_mut().zip(chunk) {
            *x ^= b;
        }
        x = key.encrypt_block(&x);
    }
    x
}
