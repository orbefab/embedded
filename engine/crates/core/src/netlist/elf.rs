//! A bare-metal program as a 32-bit little-endian Xtensa ELF executable: its entry, the bytes of
//! each loadable segment, and the code symbols a diagnostic names a PC by.

use std::ops::Range;

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
        let header = Reader(bytes);
        if bytes.get(..4) != Some(b"\x7fELF".as_slice())
            || header.u8(4) != Some(1)
            || header.u8(5) != Some(1)
            || header.u16(16) != Some(ET_EXEC)
            || header.u16(18) != Some(EM_XTENSA)
        {
            return Err(FirmwareRule::Elf);
        }
        let field = |at: usize| header.u32(at).ok_or(FirmwareRule::Elf);
        let half = |at: usize| header.u16(at).ok_or(FirmwareRule::Elf);
        let entry = field(24)?;
        let (phoff, shoff) = (field(28)? as usize, field(32)? as usize);
        let (phentsize, phnum) = (half(42)? as usize, half(44)? as usize);
        let (shentsize, shnum) = (half(46)? as usize, half(48)? as usize);

        let mut segments = Vec::new();
        for k in 0..phnum {
            let at = entry_at(phoff, k, phentsize)?;
            if phentsize < 32 || header.u32(at) != Some(PT_LOAD) {
                header.u32(at).ok_or(FirmwareRule::Elf)?;
                continue;
            }
            let offset = field(at + 4)? as usize;
            let (addr, filesz, memsz) = (field(at + 8)?, field(at + 16)?, field(at + 20)?);
            if memsz == 0 {
                continue;
            }
            let file = span(offset, filesz as usize)?;
            if filesz > memsz || bytes.get(file.clone()).is_none() {
                return Err(FirmwareRule::Elf);
            }
            segments.push(Segment { addr, memsz, file });
        }
        if segments.is_empty() {
            return Err(FirmwareRule::Elf);
        }
        check_layout(&segments, entry, windows)?;

        // An ELF file has at most one symbol table, so parse reads the first.
        let mut symtab = None;
        for k in 0..shnum {
            let at = entry_at(shoff, k, shentsize)?;
            if shentsize >= 40 && header.u32(at + 4) == Some(SHT_SYMTAB) {
                symtab = Some(at);
                break;
            }
        }
        let mut symbols = Vec::new();
        if let Some(at) = symtab {
            let (offset, size) = (field(at + 16)? as usize, field(at + 20)? as usize);
            let strings = entry_at(shoff, field(at + 24)? as usize, shentsize)?;
            let (str_offset, str_size) = (field(strings + 16)?, field(strings + 20)?);
            let strtab = span(str_offset as usize, str_size as usize)?;
            let names = bytes.get(strtab.clone()).ok_or(FirmwareRule::Elf)?;
            let table = bytes.get(span(offset, size)?).ok_or(FirmwareRule::Elf)?;
            // A name that starts past the table's last NUL has no end.
            let last_nul = names.iter().rposition(|&byte| byte == 0);
            symbols.reserve_exact(table.len() / 16);
            for entry in table.as_chunks::<16>().0 {
                let entry = Reader(entry);
                let (name, addr) = (entry.u32(0).unwrap_or(0), entry.u32(4).unwrap_or(0));
                let (info, section) = (entry.u8(12).unwrap_or(0), entry.u16(14).unwrap_or(0));
                if section == 0 || !matches!(info & 0xf, STT_NOTYPE | STT_FUNC) {
                    continue;
                }
                let name = name as usize;
                if last_nul.is_none_or(|nul| name > nul) {
                    continue;
                }
                let text = &names[name..];
                if text[0] == 0 || text[0] == b'$' || text.starts_with(b".L") {
                    continue;
                }
                symbols.push(Symbol {
                    addr,
                    name: strtab.start + name..strtab.end,
                });
            }
        }
        symbols.sort_by_key(|symbol| symbol.addr);
        symbols.dedup_by(|b, a| a.addr == b.addr);
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

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&self, at: usize) -> Option<u8> {
        self.0.get(at).copied()
    }

    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.0.get(at..at + 2)?.try_into().ok()?))
    }

    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(at..at + 4)?.try_into().ok()?))
    }
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
