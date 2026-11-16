//! A little-endian ELF32 reader for the probe images: the entry, the loadable bytes of each
//! segment, and the symbol table.

use object::Endianness;
use object::elf::FileHeader32;
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
        assert_eq!(&bytes[..4], b"\x7fELF", "ELF magic");
        assert_eq!(bytes[4], 1, "ELF32");
        assert_eq!(bytes[5], 1, "little-endian");
        let header = FileHeader32::<Endianness>::parse(bytes).expect("ELF header");
        let endian = header.endian().expect("endian");
        let entry = u32::try_from(u64::from(header.e_entry(endian))).expect("entry");
        let programs = header
            .program_headers(endian, bytes)
            .expect("program headers");
        let mut segments = Vec::new();
        for program in programs {
            if program.p_type(endian) != object::elf::PT_LOAD {
                continue;
            }
            let offset = usize::try_from(u64::from(program.p_offset(endian))).expect("offset");
            let addr = u32::try_from(u64::from(program.p_vaddr(endian))).expect("address");
            let filesz = usize::try_from(u64::from(program.p_filesz(endian))).expect("filesz");
            segments.push(Segment {
                addr,
                data: bytes[offset..offset + filesz].to_vec(),
            });
        }

        let sections = header
            .section_headers(endian, bytes)
            .expect("section headers");
        let mut symbols = Vec::new();
        for section in sections {
            if section.sh_type(endian) != object::elf::SHT_SYMTAB {
                continue;
            }
            let syms: &[object::elf::Sym32<Endianness>] =
                section.data_as_array(endian, bytes).expect("symbol table");
            let link = section.sh_link(endian) as usize;
            let strings = &sections[link];
            let str_offset =
                usize::try_from(u64::from(strings.sh_offset(endian))).expect("string offset");
            let str_size =
                usize::try_from(u64::from(strings.sh_size(endian))).expect("string size");
            let strtab = &bytes[str_offset..str_offset + str_size];
            for sym in syms {
                let name_at = sym.st_name(endian) as usize;
                let name = c_str(&strtab[name_at..]);
                if !name.is_empty() {
                    symbols.push((name, sym.st_value(endian)));
                }
            }
        }
        Elf {
            entry,
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
