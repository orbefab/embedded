//! What the display parts of a transient run showed.
//!
//! ```text
//! traces/display.csv               when the netlist has a display part
//!   t_ps,part,x0,y0,x1,y1,bytes,hash
//!   <end ps>,<part name>,<first column>,<first row>,<last column>,<last row>,<bytes>,<hash>
//! traces/display-<part>.rgb565     each pixel panel's frame memory at the run's end
//! ```
//!
//! A row is one pixel write the panel closed: its window as the column and row commands set
//! it, both ends included, the bytes it took, and the first 16 hex digits of their SHA-256. A
//! frame memory file holds each pixel's two bytes in the order the panel took them, rows top to
//! bottom, each row left to right.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::name::Name;
use crate::time::Time;

/// One closed pixel write. `part` indexes the run's display parts, in part order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rectangle {
    pub at: Time,
    pub part: u16,
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
    pub bytes: u64,
    /// The first eight bytes of the SHA-256 of the bytes.
    pub hash: [u8; 8],
}

/// The SHA-256 of a pixel write's bytes, fed as they arrive.
#[derive(Clone, Debug, Default)]
pub struct RectangleHash(Sha256);

impl RectangleHash {
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finish(self) -> [u8; 8] {
        let digest = self.0.finalize();
        let mut hash = [0; 8];
        hash.copy_from_slice(&digest[..8]);
        hash
    }
}

/// Two hashes fed the same bytes are equal.
impl PartialEq for RectangleHash {
    fn eq(&self, other: &RectangleHash) -> bool {
        self.0.clone().finalize() == other.0.clone().finalize()
    }
}

/// The state of a pixel panel a reader sees: `Session::display` returns it, and each entry of
/// `observations.json` `displays` adds the image file and its SHA-256.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Panel {
    pub kind: &'static str,
    pub width: u16,
    pub height: u16,
    /// How the frame memory file holds a pixel.
    pub format: &'static str,
    pub flags: Flags,
    /// The memory access control byte, which orders and mirrors writes.
    pub madctl: u8,
    /// The pixel format byte.
    pub colmod: u8,
    /// How many registers hold a stored last write. At most 256.
    pub registers: u64,
    /// The last duty its backlight pin took, `None` before the first.
    pub brightness: Option<Brightness>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub display_on: bool,
    pub inverted: bool,
    /// Its reset pin reads low.
    pub reset: bool,
    pub sleep: bool,
}

/// `duty` of `steps` at `hz`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Brightness {
    pub duty: u32,
    pub steps: u64,
    pub hz: u32,
}

/// One `observations.json` `displays` entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Observed {
    #[serde(flatten)]
    pub panel: Panel,
    /// The frame memory file, under the run folder.
    pub image: String,
    /// Lowercase hex.
    pub sha256: String,
}

/// A run's display parts in part order, the rows they wrote, ordered by time, then by part
/// name, then in the order each part closed them, and each part's state and frame memory at
/// the run's end.
#[derive(Debug, PartialEq)]
pub struct Displays {
    parts: Vec<(Name, Panel, Vec<u8>)>,
    rows: Vec<Rectangle>,
}

impl Displays {
    pub fn new(parts: Vec<(Name, Panel, Vec<u8>)>, rows: Vec<Rectangle>) -> Displays {
        Displays { parts, rows }
    }

    /// The bytes of `display.csv`. The header, then one row per rectangle in order, each line
    /// ended by `\n`.
    pub fn csv(&self) -> Vec<u8> {
        let mut text = String::from("t_ps,part,x0,y0,x1,y1,bytes,hash\n");
        for row in &self.rows {
            let part = self.parts[usize::from(row.part)].0.as_str();
            let hash: String = row.hash.iter().map(|byte| format!("{byte:02x}")).collect();
            writeln!(
                text,
                "{},{part},{},{},{},{},{},{hash}",
                row.at.as_ps(),
                row.x0,
                row.y0,
                row.x1,
                row.y1,
                row.bytes
            )
            .expect("a String takes every write");
        }
        text.into_bytes()
    }

    /// Each part's frame memory file, by its name under `traces/`.
    pub fn images(&self) -> impl Iterator<Item = (String, &[u8])> {
        self.parts
            .iter()
            .map(|(name, _, image)| (image_file(name), image.as_slice()))
    }

    /// The `observations.json` `displays` object, by part name.
    pub fn observed(&self) -> std::collections::BTreeMap<String, Observed> {
        self.parts
            .iter()
            .map(|(name, panel, image)| {
                let sha256: String = Sha256::digest(image)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                let observed = Observed {
                    panel: panel.clone(),
                    image: format!("traces/{}", image_file(name)),
                    sha256,
                };
                (name.as_str().to_string(), observed)
            })
            .collect()
    }
}

fn image_file(part: &Name) -> String {
    format!("display-{}.rgb565", part.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Panel {
        Panel {
            kind: "st77916",
            width: 2,
            height: 1,
            format: "rgb565",
            flags: Flags {
                display_on: true,
                inverted: false,
                reset: false,
                sleep: false,
            },
            madctl: 0,
            colmod: 0x55,
            registers: 3,
            brightness: None,
        }
    }

    #[test]
    fn a_row_is_its_window_bytes_and_hash() {
        let mut hash = RectangleHash::default();
        hash.update(b"a");
        hash.update(b"bc");
        let rows = vec![Rectangle {
            at: Time::from_ps(500),
            part: 0,
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 0,
            bytes: 3,
            hash: hash.finish(),
        }];
        let displays = Displays::new(
            vec![(Name::new("lcd1").unwrap(), panel(), vec![1, 2, 3, 4])],
            rows,
        );
        assert_eq!(
            String::from_utf8(displays.csv()).unwrap(),
            format!(
                "t_ps,part,x0,y0,x1,y1,bytes,hash\n500,lcd1,0,0,1,0,3,{}\n",
                crate::link::hash(b"abc")
            )
        );
        let images: Vec<(String, &[u8])> = displays.images().collect();
        assert_eq!(
            images,
            [("display-lcd1.rgb565".to_string(), &[1, 2, 3, 4][..])]
        );
    }

    #[test]
    fn an_entry_names_its_image_and_its_sha256() {
        let displays = Displays::new(
            vec![(Name::new("lcd1").unwrap(), panel(), b"abc".to_vec())],
            Vec::new(),
        );
        assert_eq!(displays.csv(), b"t_ps,part,x0,y0,x1,y1,bytes,hash\n");
        let json = serde_json::to_string(&displays.observed()).unwrap();
        assert_eq!(
            json,
            r#"{"lcd1":{"kind":"st77916","width":2,"height":1,"format":"rgb565","flags":{"display_on":true,"inverted":false,"reset":false,"sleep":false},"madctl":0,"colmod":85,"registers":3,"brightness":null,"image":"traces/display-lcd1.rgb565","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}"#
        );
    }
}
