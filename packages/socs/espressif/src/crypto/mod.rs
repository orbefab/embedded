//! Chip-agnostic cryptographic primitives shared across Espressif SoCs.
//!
//! These modules implement the pure mathematical operations for SHA-2 (FIPS 180-4) and RSA
//! big-integer Montgomery arithmetic, independent of any chip's register map or DMA controller.
//! AES and SHA-1 are in `orbe-crypto`.

pub(crate) mod rsa;
pub(crate) mod sha;
