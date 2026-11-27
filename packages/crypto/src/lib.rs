//! The cryptography the chip models and the virtual network share, on `std` alone: AES and its
//! block modes, CCM, AES key wrap, SHA-1, HMAC-SHA1, PBKDF2-HMAC-SHA1 and the IEEE 802.11 PRF.
//!
//! It holds only what WPA2-PSK and the ESP32-S3's AES and SHA accelerators need. Each primitive
//! is checked against the known-answer vectors of the standard that defines it, in
//! `tests/vectors`.

pub mod aes;
pub mod ccm;
pub mod hmac;
pub mod key_wrap;
pub mod pbkdf2;
pub mod prf;
pub mod sha1;
