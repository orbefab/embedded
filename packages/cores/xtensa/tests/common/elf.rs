//! A little-endian ELF32 reader for the images the decoder checks: the loadable bytes of each
//! segment.

use object::LittleEndian as Le;
use object::elf::{FileHeader32, PT_LOAD};
use object::read::elf::{FileHeader, ProgramHeader};

pub struct Elf {
    pub segments: Vec<Segment>,
}

pub struct Segment {
    pub data: Vec<u8>,
}

impl Elf {
    pub fn parse(bytes: &[u8]) -> Elf {
        let header = FileHeader32::<Le>::parse(bytes).expect("ELF32 header");
        header.endian().expect("little-endian");
        let segments = header
            .program_headers(Le, bytes)
            .expect("program headers")
            .iter()
            .filter(|program| program.p_type(Le) == PT_LOAD)
            .map(|program| Segment {
                data: program.data(Le, bytes).expect("segment bytes").to_vec(),
            })
            .collect();
        Elf { segments }
    }
}
