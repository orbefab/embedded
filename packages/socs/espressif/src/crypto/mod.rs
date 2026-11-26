//! Chip-agnostic cryptographic primitives shared across Espressif SoCs.
//!
//! These modules implement the pure mathematical operations for SHA (FIPS 180-4), AES (FIPS 197
//! and NIST SP 800-38A) and RSA big-integer Montgomery arithmetic, independent of any chip's
//! register map or DMA controller.

pub(crate) mod aes;
pub(crate) mod rsa;
pub(crate) mod sha;
