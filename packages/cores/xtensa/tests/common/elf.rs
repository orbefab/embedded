//! A little-endian ELF32 reader for the probe images: the entry, the loadable bytes of each
//! segment, and the symbol table.

use object::LittleEndian as Le;
use object::elf::{FileHeader32, PT_LOAD, SHT_SYMTAB, Sym32};
use object::read::elf::{FileHeader, ProgramHeader, SectionHeader, Sym};

pub struct Elf {
    pub entry: u32,
    pub segments: Vec<Segment>,
    symbols: Vec<(String, u32)>,
}

pub struct Segment {
    pub addr: u32,
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
                addr: program.p_vaddr(Le),
                data: program.data(Le, bytes).expect("segment bytes").to_vec(),
            })
            .collect();
        let sections = header.section_headers(Le, bytes).expect("section headers");
        let mut symbols = Vec::new();
        for section in sections.iter().filter(|s| s.sh_type(Le) == SHT_SYMTAB) {
            let syms: &[Sym32<Le>] = section.data_as_array(Le, bytes).expect("symbol table");
            let strtab = sections[section.sh_link(Le) as usize]
                .data(Le, bytes)
                .expect("string table");
            for sym in syms {
                let name = c_str(&strtab[sym.st_name(Le) as usize..]);
                if !name.is_empty() {
                    symbols.push((name, sym.st_value(Le)));
                }
            }
        }
        Elf {
            entry: header.e_entry(Le),
            segments,
            symbols,
        }
    }

    /// The `len` loaded bytes from `addr`, within one segment.
    pub fn bytes_at(&self, addr: u32, len: usize) -> &[u8] {
        let segment = self
            .segments
            .iter()
            .find(|segment| {
                addr >= segment.addr && (addr - segment.addr) as usize + len <= segment.data.len()
            })
            .unwrap_or_else(|| panic!("no segment holds {len} bytes at {addr:#010x}"));
        let at = (addr - segment.addr) as usize;
        &segment.data[at..at + len]
    }

    pub fn symbol(&self, name: &str) -> u32 {
        self.symbols
            .iter()
            .find(|(sym, _)| sym == name)
            .unwrap_or_else(|| panic!("symbol {name} is in the image"))
            .1
    }
}

fn c_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap();
    String::from_utf8(bytes[..end].to_vec()).unwrap()
}
