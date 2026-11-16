//! A bare-metal program as a 32-bit little-endian Xtensa ELF executable: its entry, the bytes of
//! each loadable segment, and the code symbols a diagnostic names a PC by.

use std::ops::Range;

use object::Endianness;
use object::elf::FileHeader32;
use object::read::ReadRef;
use object::read::elf::{FileHeader, ProgramHeader, SectionHeader, Sym};

use crate::blob::Blob;
use crate::diagnostic::FirmwareRule;

const EM_XTENSA: u16 = 94;
const ET_EXEC: u16 = 2;
const PT_LOAD: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const STT_NOTYPE: u8 = 0;
const STT_FUNC: u8 = 2;

/// A parsed executable. Only parse builds one, so every loadable segment lies in the memory its
/// kind names, no two share a byte there, and the entry is in one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elf {
    blob: Blob,
    entry: u32,
    segments: Vec<Segment>,
    /// By address, then in table order. The first at an address names it.
    symbols: Vec<Symbol>,
}

/// A loadable segment: `memsz` bytes at `addr`, the first of them from the file. The rest are
/// zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub addr: u32,
    pub memsz: u32,
    file: Range<usize>,
}

/// A code symbol: a function, or a label the assembler left untyped. Its name is the bytes of
/// the string table from `name.start` up to the first NUL, and the table ends at `name.end`. The
/// name stays in the blob, so the symbols of a table hold at most half again its bytes, however
/// long their names.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Symbol {
    addr: u32,
    name: Range<usize>,
}

/// Where a segment may load: `[start, end)` on one bus, which is `[start - base + offset, ...)`
/// of the memory behind it. Two windows onto one memory share bytes where those offsets meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub start: u32,
    pub end: u32,
    pub offset: u32,
    /// Whether code runs from here, so the entry may lie in it.
    pub code: bool,
}

impl Elf {
    /// Reads `blob` as an executable for memory `windows`.
    pub fn parse(blob: Blob, windows: &[Window]) -> Result<Elf, FirmwareRule> {
        let bytes = &blob.bytes[..];
        let header = FileHeader32::<Endianness>::parse(bytes).map_err(|_| FirmwareRule::Elf)?;
        let endian = header.endian().map_err(|_| FirmwareRule::Elf)?;
        if !header.is_little_endian()
            || header.e_type(endian).0 != ET_EXEC
            || header.e_machine(endian).0 != EM_XTENSA
        {
            return Err(FirmwareRule::Elf);
        }
        let entry = narrow(header.e_entry(endian))?;
        let segments = segments(header, endian, bytes)?;
        check_layout(&segments, entry, windows)?;
        let symbols = symbols(header, endian, bytes)?;
        Ok(Elf {
            blob,
            entry,
            segments,
            symbols,
        })
    }

    pub fn blob(&self) -> &Blob {
        &self.blob
    }

    pub fn entry(&self) -> u32 {
        self.entry
    }

    /// In the file's program header order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The bytes of `segment` the file holds, `filesz` of its `memsz`.
    pub fn data(&self, segment: &Segment) -> &[u8] {
        &self.blob.bytes[segment.file.clone()]
    }

    /// The symbol at or below `pc`, with the offset from it, as `name+0x1c`. `None` below every
    /// symbol.
    pub fn symbolize(&self, pc: u32) -> Option<String> {
        let k = self.symbols.partition_point(|symbol| symbol.addr <= pc);
        let symbol = self.symbols.get(k.checked_sub(1)?)?;
        let name = self.name(symbol);
        let name = String::from_utf8_lossy(&name[..name.iter().position(|&byte| byte == 0)?]);
        Some(match pc - symbol.addr {
            0 => name.into_owned(),
            offset => format!("{name}+{offset:#x}"),
        })
    }

    /// The string table from `symbol`'s name to the table's end.
    fn name(&self, symbol: &Symbol) -> &[u8] {
        &self.blob.bytes[symbol.name.clone()]
    }
}

/// Each segment lies in one window, no two reach the same byte of memory, and the entry lies in
/// a segment in a code window.
fn check_layout(segments: &[Segment], entry: u32, windows: &[Window]) -> Result<(), FirmwareRule> {
    let mut reached: Vec<Range<u64>> = Vec::new();
    let mut entry_mapped = false;
    for segment in segments {
        let (start, end) = (
            segment.addr,
            u64::from(segment.addr) + u64::from(segment.memsz),
        );
        let window = windows
            .iter()
            .find(|w| w.start <= start && end <= u64::from(w.end))
            .ok_or(FirmwareRule::Unmapped)?;
        let at = u64::from(start - window.start + window.offset);
        let range = at..at + u64::from(segment.memsz);
        if reached
            .iter()
            .any(|seen| seen.start < range.end && range.start < seen.end)
        {
            return Err(FirmwareRule::Overlap);
        }
        reached.push(range);
        entry_mapped |= window.code && start <= entry && u64::from(entry) < end;
    }
    if !entry_mapped {
        return Err(FirmwareRule::Unmapped);
    }
    Ok(())
}

/// Loadable segments, in program-header order. `object` reads a program header only when
/// `e_phentsize` is 32 and `e_phoff` is not 0, so a wider entry or a table at offset 0 is
/// refused. The count is the raw `e_phnum`, including `PN_XNUM`.
fn segments(
    header: &FileHeader32<Endianness>,
    endian: Endianness,
    bytes: &[u8],
) -> Result<Vec<Segment>, FirmwareRule> {
    let phoff = usize_word(header.e_phoff(endian))?;
    let phentsize = usize::from(header.e_phentsize(endian));
    let phnum = usize::from(header.e_phnum(endian));
    if phnum > 0 {
        entry_at(phoff, phnum - 1, phentsize)?;
    }
    if header.e_phnum(endian) == object::elf::PN_XNUM || phentsize != 32 || phoff == 0 {
        return Err(FirmwareRule::Elf);
    }
    let programs = header
        .program_headers(endian, bytes)
        .map_err(|_| FirmwareRule::Elf)?;
    let mut segments = Vec::new();
    for program in programs {
        if program.p_type(endian).0 != PT_LOAD {
            continue;
        }
        let memsz = narrow(program.p_memsz(endian))?;
        if memsz == 0 {
            continue;
        }
        let filesz = narrow(program.p_filesz(endian))?;
        let offset = usize_word(program.p_offset(endian))?;
        let addr = narrow(program.p_vaddr(endian))?;
        let file = span(offset, filesz as usize)?;
        if filesz > memsz || bytes.get(file.clone()).is_none() {
            return Err(FirmwareRule::Elf);
        }
        segments.push(Segment { addr, memsz, file });
    }
    if segments.is_empty() {
        return Err(FirmwareRule::Elf);
    }
    Ok(segments)
}

/// The first symbol table's code symbols. `e_shnum` 0 means there are none: the extended count
/// in section 0 is not a table this loader reads. A header size other than 40 yields none. A
/// table `object` will not return whole still yields a symbol table whose own header is in the
/// file. A name is the string table from the name through the table's end, so the symbols hold
/// no copy of it.
fn symbols(
    header: &FileHeader32<Endianness>,
    endian: Endianness,
    bytes: &[u8],
) -> Result<Vec<Symbol>, FirmwareRule> {
    let shoff = usize_word(header.e_shoff(endian))?;
    let shentsize = usize::from(header.e_shentsize(endian));
    let shnum = usize::from(header.e_shnum(endian));
    if shnum > 0 {
        entry_at(shoff, shnum - 1, shentsize)?;
    }
    if shentsize != 40 || shnum == 0 {
        return Ok(Vec::new());
    }
    let mut symtab = None;
    for k in 0..shnum {
        let at = entry_at(shoff, k, shentsize)?;
        let Ok(section) = bytes.read_at::<object::elf::SectionHeader32<Endianness>>(at as u64)
        else {
            continue;
        };
        if section.sh_type(endian).0 == SHT_SYMTAB {
            symtab = Some(*section);
            break;
        }
    }
    let Some(symtab) = symtab else {
        return Ok(Vec::new());
    };
    let offset = usize_word(symtab.sh_offset(endian))?;
    let size = usize_word(symtab.sh_size(endian))?;
    if bytes.get(span(offset, size)?).is_none() {
        return Err(FirmwareRule::Elf);
    }
    let link = usize::try_from(symtab.sh_link(endian)).map_err(|_| FirmwareRule::Elf)?;
    let strings = entry_at(shoff, link, shentsize)?;
    let strings = bytes
        .read_at::<object::elf::SectionHeader32<Endianness>>(strings as u64)
        .map_err(|_| FirmwareRule::Elf)?;
    let str_offset = usize_word(strings.sh_offset(endian))?;
    let str_size = usize_word(strings.sh_size(endian))?;
    let strtab = span(str_offset, str_size)?;
    let names = bytes.get(strtab.clone()).ok_or(FirmwareRule::Elf)?;
    let width = std::mem::size_of::<object::elf::Sym32<Endianness>>();
    let count = size / width;
    let table = bytes
        .read_slice_at::<object::elf::Sym32<Endianness>>(offset as u64, count)
        .map_err(|_| FirmwareRule::Elf)?;
    // A name that starts past the table's last NUL has no end.
    let last_nul = names.iter().rposition(|&byte| byte == 0);
    let mut symbols = Vec::new();
    symbols.reserve_exact(count);
    for symbol in table {
        if symbol.st_shndx(endian) == object::elf::SHN_UNDEF
            || !matches!(symbol.st_type().0, STT_NOTYPE | STT_FUNC)
        {
            continue;
        }
        let name = symbol.st_name(endian) as usize;
        if last_nul.is_none_or(|nul| name > nul) {
            continue;
        }
        let text = &names[name..];
        if text[0] == 0 || text[0] == b'$' || text.starts_with(b".L") {
            continue;
        }
        symbols.push(Symbol {
            addr: symbol.st_value(endian),
            name: strtab.start + name..strtab.end,
        });
    }
    symbols.sort_by_key(|symbol| symbol.addr);
    symbols.dedup_by(|later, earlier| earlier.addr == later.addr);
    Ok(symbols)
}

fn narrow(word: impl Into<u64>) -> Result<u32, FirmwareRule> {
    u32::try_from(word.into()).map_err(|_| FirmwareRule::Elf)
}

fn usize_word(word: impl Into<u64>) -> Result<usize, FirmwareRule> {
    usize::try_from(word.into()).map_err(|_| FirmwareRule::Elf)
}

/// Where entry `k` of a table at `offset` with entries of `size` bytes starts. A table that runs
/// past the address space runs past the file.
fn entry_at(offset: usize, k: usize, size: usize) -> Result<usize, FirmwareRule> {
    k.checked_mul(size)
        .and_then(|at| at.checked_add(offset))
        .and_then(|at| at.checked_add(40))
        .map(|end| end - 40)
        .ok_or(FirmwareRule::Elf)
}

/// `len` bytes from `offset`.
fn span(offset: usize, len: usize) -> Result<Range<usize>, FirmwareRule> {
    let end = offset.checked_add(len).ok_or(FirmwareRule::Elf)?;
    Ok(offset..end)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blob::{Role, Sha256};

    pub(crate) const WINDOWS: [Window; 2] = [
        Window {
            start: 0x4000,
            end: 0x5000,
            offset: 0,
            code: true,
        },
        Window {
            start: 0x1000,
            end: 0x2000,
            offset: 0x800,
            code: false,
        },
    ];

    /// An executable with one segment per `(addr, bytes, memsz)` and one function symbol per
    /// `(name, addr)`.
    pub(crate) fn build(
        entry: u32,
        segments: &[(u32, &[u8], u32)],
        symbols: &[(&str, u32)],
    ) -> Vec<u8> {
        let phoff = 52;
        let data_at = phoff + 32 * segments.len();
        let mut data = Vec::new();
        let mut headers = Vec::new();
        for &(addr, bytes, memsz) in segments {
            let offset = (data_at + data.len()) as u32;
            for word in [PT_LOAD, offset, addr, addr, bytes.len() as u32, memsz, 7, 4] {
                headers.extend(word.to_le_bytes());
            }
            data.extend_from_slice(bytes);
        }
        let mut strings = vec![0u8];
        let mut table = vec![0u8; 16];
        for &(name, addr) in symbols {
            let at = strings.len() as u32;
            strings.extend(name.as_bytes());
            strings.push(0);
            table.extend(at.to_le_bytes());
            table.extend(addr.to_le_bytes());
            table.extend(0u32.to_le_bytes());
            table.extend([0x12, 0]);
            table.extend(1u16.to_le_bytes());
        }
        let mut file = vec![0u8; 52];
        file[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        file[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
        file[18..20].copy_from_slice(&EM_XTENSA.to_le_bytes());
        file[24..28].copy_from_slice(&entry.to_le_bytes());
        file[28..32].copy_from_slice(&(phoff as u32).to_le_bytes());
        file.extend(headers);
        file.extend(data);
        let table_at = file.len() as u32;
        file.extend(&table);
        let strings_at = file.len() as u32;
        file.extend(&strings);
        let shoff = file.len() as u32;
        file[32..36].copy_from_slice(&shoff.to_le_bytes());
        file[42..44].copy_from_slice(&32u16.to_le_bytes());
        file[44..46].copy_from_slice(&(segments.len() as u16).to_le_bytes());
        file[46..48].copy_from_slice(&40u16.to_le_bytes());
        file[48..50].copy_from_slice(&3u16.to_le_bytes());
        let section = |kind: u32, offset: u32, size: u32, link: u32| {
            let mut header = vec![0u8; 40];
            header[4..8].copy_from_slice(&kind.to_le_bytes());
            header[16..20].copy_from_slice(&offset.to_le_bytes());
            header[20..24].copy_from_slice(&size.to_le_bytes());
            header[24..28].copy_from_slice(&link.to_le_bytes());
            header
        };
        file.extend(section(0, 0, 0, 0));
        file.extend(section(SHT_SYMTAB, table_at, table.len() as u32, 2));
        file.extend(section(3, strings_at, strings.len() as u32, 0));
        file
    }

    /// `symbols` function symbols from `0x4000` on that all name one string of `name` bytes, in
    /// one symbol table that `tables` section headers each point at.
    fn hostile(symbols: u32, name: usize, tables: u32) -> Vec<u8> {
        let one = build(0x4000, &[(0x4000, &[0; 4], 4)], &[]);
        let mut file = one[..52 + 32 + 4].to_vec();
        let strings_at = file.len() as u32;
        file.push(0);
        file.extend(std::iter::repeat_n(b'f', name));
        file.push(0);
        let table_at = file.len() as u32;
        file.extend([0; 16]);
        for k in 0..symbols {
            file.extend(1u32.to_le_bytes());
            file.extend((0x4000 + k).to_le_bytes());
            file.extend(0u32.to_le_bytes());
            file.extend([0x12, 0]);
            file.extend(1u16.to_le_bytes());
        }
        let shoff = file.len() as u32;
        file[32..36].copy_from_slice(&shoff.to_le_bytes());
        file[48..50].copy_from_slice(&(tables as u16 + 2).to_le_bytes());
        let section = |kind: u32, offset: u32, size: u32, link: u32| {
            let mut header = [0u8; 40];
            header[4..8].copy_from_slice(&kind.to_le_bytes());
            header[16..20].copy_from_slice(&offset.to_le_bytes());
            header[20..24].copy_from_slice(&size.to_le_bytes());
            header[24..28].copy_from_slice(&link.to_le_bytes());
            header
        };
        file.extend(section(0, 0, 0, 0));
        for _ in 0..tables {
            file.extend(section(
                SHT_SYMTAB,
                table_at,
                16 * (symbols + 1),
                tables + 1,
            ));
        }
        file.extend(section(3, strings_at, name as u32 + 2, 0));
        file
    }

    fn parse(file: Vec<u8>) -> Result<Elf, FirmwareRule> {
        let bytes: Arc<[u8]> = file.into();
        let blob = Blob {
            role: Role::Elf,
            sha256: Sha256::of(&bytes),
            bytes,
        };
        Elf::parse(blob, &WINDOWS)
    }

    #[test]
    fn segments_entry_and_symbols_read_back() {
        let file = build(
            0x4004,
            &[(0x4000, &[1, 2, 3, 4, 5, 6, 7, 8], 8), (0x1100, &[9], 16)],
            &[("start", 0x4004), ("main", 0x4000)],
        );
        let elf = parse(file).unwrap();
        assert_eq!(elf.entry(), 0x4004);
        let segments = elf.segments();
        assert_eq!(segments.len(), 2);
        assert_eq!(elf.data(&segments[0]), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!((segments[1].addr, segments[1].memsz), (0x1100, 16));
        assert_eq!(elf.data(&segments[1]), &[9]);
        assert_eq!(elf.symbolize(0x4000).as_deref(), Some("main"));
        assert_eq!(elf.symbolize(0x4007).as_deref(), Some("start+0x3"));
        assert_eq!(elf.symbolize(0x3fff), None);
    }

    #[test]
    fn symbols_hold_at_most_half_again_their_table_whatever_their_names() {
        for (count, name) in [(2048, 16 * 1024), (2048, 1), (1, 16 * 1024)] {
            let elf = parse(hostile(count, name, 4)).unwrap();
            let table = 16 * (count as usize + 1);
            let blob = elf.blob.bytes.as_ptr_range();
            let names: usize = elf
                .symbols
                .iter()
                .map(|symbol| elf.name(symbol))
                .filter(|name| !blob.contains(&name.as_ptr()))
                .map(<[u8]>::len)
                .sum();
            let held = elf.symbols.capacity() * std::mem::size_of::<Symbol>() + names;
            assert!(
                held <= 3 * table / 2 + 64,
                "{held} bytes of symbols for a {table}-byte table of {name}-byte names"
            );
            let text = "f".repeat(name);
            assert_eq!(elf.symbolize(0x4000).as_deref(), Some(text.as_str()));
            assert_eq!(
                elf.symbolize(0x4000 + count - 1 + 5),
                Some(format!("{text}+0x5"))
            );
        }
    }

    #[test]
    fn a_file_that_is_not_an_xtensa_executable_is_refused() {
        let good = build(0x4000, &[(0x4000, &[0; 4], 4)], &[]);
        let mut wrong_machine = good.clone();
        wrong_machine[18] = 3;
        assert_eq!(parse(wrong_machine), Err(FirmwareRule::Elf));
        assert_eq!(parse(good[..60].to_vec()), Err(FirmwareRule::Elf));
        assert_eq!(parse(b"MZ".to_vec()), Err(FirmwareRule::Elf));
    }

    #[test]
    fn a_segment_outside_memory_or_on_shared_bytes_is_refused() {
        let outside = build(0x4000, &[(0x4000, &[0; 4], 4), (0x2ffc, &[0; 4], 8)], &[]);
        assert_eq!(parse(outside), Err(FirmwareRule::Unmapped));
        // 0x1000 on the second window is byte 0x800 of the memory, as 0x4800 is on the first.
        let shared = build(
            0x4000,
            &[(0x4000, &[0; 4], 0x804), (0x1000, &[0; 4], 4)],
            &[],
        );
        assert_eq!(parse(shared), Err(FirmwareRule::Overlap));
        let entry_in_data = build(0x1000, &[(0x4000, &[0; 4], 4), (0x1000, &[0; 4], 4)], &[]);
        assert_eq!(parse(entry_in_data), Err(FirmwareRule::Unmapped));
    }
}
