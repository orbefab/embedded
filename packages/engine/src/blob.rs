//! Blobs: input files a netlist names by SHA-256 instead of holding them, such as a chip's
//! firmware. A run looks each one up by its hash in the [`Blobs`] its caller gives, and the
//! netlist stays small enough to copy into every run folder.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256 as Hasher};

/// A SHA-256 digest. It reads and prints as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    pub fn of(bytes: &[u8]) -> Sha256 {
        Sha256(Hasher::digest(bytes).into())
    }

    /// 64 lowercase hex digits, and nothing else.
    pub fn parse(text: &str) -> Option<Sha256> {
        let digits = text.as_bytes();
        if digits.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (byte, pair) in out.iter_mut().zip(digits.as_chunks::<2>().0) {
            *byte = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Some(Sha256(out))
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({self})")
    }
}

/// The blobs a caller offers a run, each under the hash it was filed by. A file named by a hash
/// it does not have is kept under that name, so the netlist error can say so.
#[derive(Clone, Debug, Default)]
pub struct Blobs {
    by_name: BTreeMap<Sha256, Arc<[u8]>>,
    source: Source,
}

/// Where a caller found its blobs, so a netlist error can say where to put one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// A list the caller passed, such as the wasm package's.
    #[default]
    List,
    /// The `blobs` folder beside the netlist and each `--blobs` folder, as `orbe-run run` searches.
    Folders,
}

impl Blobs {
    /// An empty list.
    pub fn new() -> Blobs {
        Blobs::default()
    }

    /// None yet, from the folders `orbe-run run` searches.
    pub fn from_folders() -> Blobs {
        Blobs {
            source: Source::Folders,
            ..Blobs::default()
        }
    }

    pub fn source(&self) -> Source {
        self.source
    }

    /// Files `bytes` under `name`, which should be their SHA-256. A second file under one name
    /// replaces the first.
    pub fn insert(&mut self, name: Sha256, bytes: Arc<[u8]>) {
        self.by_name.insert(name, bytes);
    }

    /// Files `bytes` under their own SHA-256, and returns it.
    pub fn add(&mut self, bytes: Arc<[u8]>) -> Sha256 {
        let name = Sha256::of(&bytes);
        self.insert(name, bytes);
        name
    }

    pub fn get(&self, name: &Sha256) -> Option<&Arc<[u8]>> {
        self.by_name.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Each blob under the hash it was filed by.
    pub fn iter(&self) -> impl Iterator<Item = (Sha256, &[u8])> + '_ {
        self.by_name
            .iter()
            .map(|(&name, bytes)| (name, bytes.as_ref()))
    }
}

/// What a blob is to the part that names it. `manifest.json` lists each blob with its role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// The mask ROM as its ELF.
    Rom,
    /// A bare-metal program as an ELF executable.
    Elf,
    /// Bytes of the flash from this offset.
    Flash(u32),
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Rom => "rom",
            Role::Elf => "elf",
            Role::Flash(_) => "flash",
        }
    }

    /// Where a flash region starts.
    pub fn offset(self) -> Option<u32> {
        match self {
            Role::Flash(offset) => Some(offset),
            Role::Rom | Role::Elf => None,
        }
    }
}

/// A blob a netlist names, found and checked: its bytes hash to `sha256`.
#[derive(Clone, PartialEq, Eq)]
pub struct Blob {
    pub role: Role,
    pub sha256: Sha256,
    pub bytes: Arc<[u8]>,
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("role", &self.role)
            .field("sha256", &self.sha256)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_reads_back_what_it_prints() {
        let digest = Sha256::of(b"abc");
        let text = digest.to_string();
        assert_eq!(
            text,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(Sha256::parse(&text), Some(digest));
    }

    #[test]
    fn a_digest_is_64_lowercase_hex_digits() {
        let text = Sha256::of(b"abc").to_string();
        assert_eq!(Sha256::parse(&text.to_uppercase()), None);
        assert_eq!(Sha256::parse(&text[1..]), None);
        assert_eq!(Sha256::parse(&format!("{text}0")), None);
        assert_eq!(Sha256::parse(&text.replace('b', "g")), None);
    }
}
