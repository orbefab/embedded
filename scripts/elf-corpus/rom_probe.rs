//! Corpus dump for `rom::load`. `scripts/elf-corpus.sh` copies this in, runs it, and removes it.

use sha2::{Digest, Sha256};

#[test]
fn corpus() {
    let dir = std::env::var("ELF_CORPUS_DIR").unwrap();
    let mut inputs = 0u64;
    for path in files(&dir) {
        let bytes = std::fs::read(&path).unwrap();
        let key = path.file_name().unwrap().to_str().unwrap().to_string();
        println!("rom {key} {}", load_line(&bytes));
        inputs += 1;
    }
    let mut fixtures = files(&std::env::var("ELF_CORPUS_FIXTURES").unwrap());
    fixtures.retain(|path| path.extension().is_some_and(|ext| ext == "elf"));
    for path in fixtures {
        let bytes = std::fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        println!("rom fix/{name} {}", load_line(&bytes));
        inputs += 1;
        let mut flipped = bytes.clone();
        for (index, byte) in bytes.iter().enumerate() {
            flipped[index] = byte ^ 0xff;
            println!("rom flip/{name}/{index:08} {}", load_line(&flipped));
            flipped[index] = *byte;
            inputs += 1;
        }
    }
    // The pinned ROM is loaded once.
    if let Ok(rom) = std::env::var("ELF_CORPUS_ROM") {
        if !rom.is_empty() {
            let path = std::path::PathBuf::from(rom);
            let bytes = std::fs::read(&path).unwrap();
            let name = path.file_name().unwrap().to_str().unwrap();
            println!("rom fix/{name} {}", load_line(&bytes));
            inputs += 1;
        }
    }
    eprintln!("rom-corpus inputs {inputs}");
}

fn files(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    out.sort();
    out
}

fn load_line(bytes: &[u8]) -> String {
    match super::rom::load(bytes, |addr, len| {
        super::soc::Soc::sram_offset(addr, false).is_some()
            && super::soc::Soc::sram_offset(addr + len as u32 - 1, false).is_some()
    }) {
        Ok(image) => {
            let rom_hex = if image.rom.iter().any(|&byte| byte != 0) {
                hash(image.rom.as_ref())
            } else {
                untouched_rom(image.rom.len())
            };
            let mut data = Sha256::new();
            for (addr, bytes) in &image.data {
                data.update(addr.to_le_bytes());
                data.update((bytes.len() as u32).to_le_bytes());
                data.update(bytes);
            }
            format!("OK rom={rom_hex} data={}", hex(&data.finalize()))
        }
        Err(err) => format!("ERR {}", err.replace(['\n', '\r'], " ")),
    }
}

/// SHA-256 of `len` zero bytes. An untouched ROM image is that buffer, so the line
/// matches hashing it, and the corpus does not hash 384 KB on every flipped byte.
fn untouched_rom(len: usize) -> String {
    use std::sync::OnceLock;
    static CACHED: OnceLock<(usize, String)> = OnceLock::new();
    if let Some((cached, hex)) = CACHED.get() {
        if *cached == len {
            return hex.clone();
        }
    }
    let hex = hash(&vec![0u8; len]);
    let _ = CACHED.set((len, hex.clone()));
    hex
}

fn hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
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
