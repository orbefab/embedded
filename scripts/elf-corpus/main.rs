//! Prints one diagnostic line per ELF for the engine loader and the AVR loader.
//!
//! Usage: elf-corpus <cases-dir> <fixtures-dir> [rom-elf]
//!
//! Each file in `cases-dir` is one input, named by its file name. Each `*.elf` in
//! `fixtures-dir` is loaded whole, then again with every byte flipped (`xor 0xff`), one
//! input per byte. `rom-elf`, when given, is the pinned ROM image.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use orbe_engine::blob::{Blob, Role, Sha256};
use orbe_engine::diagnostic::FirmwareRule;
use orbe_engine::mcu::Esp32s3;
use orbe_engine::netlist::elf::Elf;
use orbe_microchip::parse_elf;
use sha2::{Digest, Sha256 as Hasher};

fn main() {
    let mut args = std::env::args().skip(1);
    let cases = args.next().expect("cases dir");
    let fixtures = args.next().expect("fixtures dir");
    let rom = args.next();
    let mut inputs = 0u64;
    for path in files(&cases) {
        let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let key = path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        emit(&key, &bytes);
        inputs += 1;
    }
    let mut fixtures = files(&fixtures);
    fixtures.retain(|path| path.extension().is_some_and(|ext| ext == "elf"));
    for path in fixtures {
        let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let name = path.file_name().unwrap().to_str().unwrap();
        emit(&format!("fix/{name}"), &bytes);
        inputs += 1;
        let mut flipped = bytes.clone();
        for (index, byte) in bytes.iter().enumerate() {
            flipped[index] = byte ^ 0xff;
            emit(&format!("flip/{name}/{index:08}"), &flipped);
            flipped[index] = *byte;
            inputs += 1;
        }
    }
    // The pinned ROM is loaded once. Flipping each of its bytes is a different test
    // from the fixture fuzz, and each flip builds a whole image.
    if let Some(rom) = rom {
        let path = PathBuf::from(rom);
        let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let name = path.file_name().unwrap().to_str().unwrap();
        emit(&format!("fix/{name}"), &bytes);
        inputs += 1;
    }
    eprintln!("elf-corpus inputs {inputs}");
}

fn files(dir: &str) -> Vec<PathBuf> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {dir}: {err}"))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    out.sort();
    out
}

fn emit(key: &str, bytes: &[u8]) {
    println!("engine {key} {}", engine(bytes));
    println!("avr {key} {}", avr(bytes));
}

fn engine(bytes: &[u8]) -> String {
    let owned: Arc<[u8]> = bytes.into();
    let blob = Blob {
        role: Role::Elf,
        sha256: Sha256::of(&owned),
        bytes: owned,
    };
    match Elf::parse(blob, &Esp32s3::RAM) {
        Ok(elf) => {
            let mut hasher = Hasher::new();
            hasher.update(elf.entry().to_le_bytes());
            for segment in elf.segments() {
                hasher.update(segment.addr.to_le_bytes());
                hasher.update(segment.memsz.to_le_bytes());
                hasher.update(elf.data(segment));
            }
            let symbol = elf.symbolize(elf.entry());
            format!("OK {} sym={}", hex(&hasher.finalize()), symbol.as_deref().unwrap_or("-"))
        }
        Err(rule) => format!("ERR {}", rule_name(rule)),
    }
}

fn rule_name(rule: FirmwareRule) -> &'static str {
    match rule {
        FirmwareRule::Elf => "elf",
        FirmwareRule::Unmapped => "unmapped",
        FirmwareRule::Overlap => "overlap",
        FirmwareRule::Record => "record",
        FirmwareRule::Checksum => "checksum",
        FirmwareRule::RecordType => "record_type",
        FirmwareRule::AfterEnd => "after_end",
        FirmwareRule::NoEnd => "no_end",
        FirmwareRule::PastFlash => "past_flash",
        FirmwareRule::Region => "region",
        FirmwareRule::Image => "image",
        FirmwareRule::Rom => "rom",
    }
}

fn avr(bytes: &[u8]) -> String {
    match parse_elf(bytes) {
        Ok(image) => {
            let mut hasher = Hasher::new();
            hasher.update(image.bytes());
            let notes: Vec<_> = image
                .notes()
                .iter()
                .map(|note| format!("{:?}:{:#x}:{}", note.region, note.paddr, note.filesz))
                .collect();
            format!("OK {} notes={}", hex(&hasher.finalize()), notes.join(","))
        }
        Err(err) => format!("ERR {}", one_line(&err.to_string())),
    }
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}
