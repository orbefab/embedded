//! A little-endian ELF32 reader for the probe images: the entry, the loadable bytes of each
//! segment, and the symbol table.

pub struct Elf {
    pub entry: u32,
    pub segments: Vec<Segment>,
    symbols: Vec<(String, u32)>,
}

pub struct Segment {
    pub addr: u32,
    pub data: Vec<u8>,
}

const PT_LOAD: u32 = 1;
const SHT_SYMTAB: u32 = 2;

impl Elf {
    pub fn parse(bytes: &[u8]) -> Elf {
        assert_eq!(&bytes[..4], b"\x7fELF", "ELF magic");
        assert_eq!(bytes[4], 1, "ELF32");
        assert_eq!(bytes[5], 1, "little-endian");
        let entry = u32_at(bytes, 24);
        let phoff = u32_at(bytes, 28) as usize;
        let shoff = u32_at(bytes, 32) as usize;
        let phentsize = u16_at(bytes, 42) as usize;
        let phnum = u16_at(bytes, 44) as usize;
        let shentsize = u16_at(bytes, 46) as usize;
        let shnum = u16_at(bytes, 48) as usize;

        let mut segments = Vec::new();
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if u32_at(bytes, ph) != PT_LOAD {
                continue;
            }
            let offset = u32_at(bytes, ph + 4) as usize;
            let addr = u32_at(bytes, ph + 8);
            let filesz = u32_at(bytes, ph + 16) as usize;
            segments.push(Segment {
                addr,
                data: bytes[offset..offset + filesz].to_vec(),
            });
        }

        let mut symbols = Vec::new();
        for i in 0..shnum {
            let sh = shoff + i * shentsize;
            if u32_at(bytes, sh + 4) != SHT_SYMTAB {
                continue;
            }
            let offset = u32_at(bytes, sh + 16) as usize;
            let size = u32_at(bytes, sh + 20) as usize;
            let link = u32_at(bytes, sh + 24) as usize;
            let entsize = u32_at(bytes, sh + 36) as usize;
            let strtab = u32_at(bytes, shoff + link * shentsize + 16) as usize;
            for sym in (offset..offset + size).step_by(entsize) {
                let name = c_str(&bytes[strtab + u32_at(bytes, sym) as usize..]);
                if !name.is_empty() {
                    symbols.push((name, u32_at(bytes, sym + 4)));
                }
            }
        }
        Elf {
            entry,
            segments,
            symbols,
        }
    }

    pub fn symbol(&self, name: &str) -> u32 {
        self.symbols
            .iter()
            .find(|(sym, _)| sym == name)
            .unwrap_or_else(|| panic!("symbol {name} is in the image"))
            .1
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn c_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap();
    String::from_utf8(bytes[..end].to_vec()).unwrap()
}
