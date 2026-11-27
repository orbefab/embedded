//! HMAC-SHA1 (RFC 2104).

use crate::sha1::{Sha1, sha1};

/// HMAC-SHA1 keyed once. A clone carries the keyed state, so PBKDF2 and the PRF hash the key
/// only once however many MACs they take.
#[derive(Clone, Debug)]
pub struct HmacSha1 {
    inner: Sha1,
    outer: Sha1,
}

impl HmacSha1 {
    pub fn new(key: &[u8]) -> HmacSha1 {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..20].copy_from_slice(&sha1(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha1::new();
        inner.update(&block.map(|b| b ^ 0x36));
        let mut outer = Sha1::new();
        outer.update(&block.map(|b| b ^ 0x5c));
        HmacSha1 { inner, outer }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finish(self) -> [u8; 20] {
        let mut outer = self.outer;
        outer.update(&self.inner.finish());
        outer.finish()
    }
}

/// HMAC-SHA1 of `data` under `key`.
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut mac = HmacSha1::new(key);
    mac.update(data);
    mac.finish()
}
