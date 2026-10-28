//! The per-chip setup. One zero-sized type per chip, so `Core<C>` monomorphizes each chip's
//! constants and decoder, and the instruction loop has no per-op chip branch.

use crate::decode::{self, Decoded};

/// A chip's core configuration. The constants come from that chip's `core-isa.h`. The decoder
/// comes from that chip's generated table and names only that chip's ops.
pub trait Config: 'static {
    const NAME: &'static str;
    /// `XCHAL_EXCM_LEVEL`. `PS.EXCM` masks interrupts up to this level.
    const EXCM_LEVEL: u32;
    /// `XCHAL_DEBUGLEVEL`. Debug exceptions are taken at this level.
    const DEBUG_LEVEL: u32;

    /// The instruction length the first byte names, or `None` when it starts no format.
    fn insn_len(first: u8) -> Option<u8>;
    /// Decodes the instruction at the front of `bytes`, whose first byte is at `pc`. An empty
    /// slice is `None`.
    fn decode(bytes: &[u8], pc: u32) -> Option<Decoded>;
    /// The text GNU objdump prints for the instruction, without the address.
    fn render(decoded: &Decoded) -> String;
}

/// The classic ESP32. Its `Op`s are the 513 of the classic table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esp32 {}

/// The ESP32-S3. Its `Op`s are the 753 of the S3 table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Esp32s3 {}

impl Config for Esp32 {
    const NAME: &'static str = "esp32";
    const EXCM_LEVEL: u32 = 3;
    const DEBUG_LEVEL: u32 = 6;

    fn insn_len(first: u8) -> Option<u8> {
        decode::classic::len(first)
    }

    fn decode(bytes: &[u8], pc: u32) -> Option<Decoded> {
        decode::classic::decode(bytes, pc)
    }

    fn render(decoded: &Decoded) -> String {
        decode::classic::render(decoded)
    }
}

impl Config for Esp32s3 {
    const NAME: &'static str = "esp32s3";
    const EXCM_LEVEL: u32 = 3;
    const DEBUG_LEVEL: u32 = 6;

    fn insn_len(first: u8) -> Option<u8> {
        decode::s3::len(first)
    }

    fn decode(bytes: &[u8], pc: u32) -> Option<Decoded> {
        decode::s3::decode(bytes, pc)
    }

    fn render(decoded: &Decoded) -> String {
        decode::s3::render(decoded)
    }
}
