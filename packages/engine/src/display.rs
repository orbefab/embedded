//! What the display parts of a transient run showed.
//!
//! ```text
//! traces/display.csv               when the netlist has a display part
//!   t_ps,part,x0,y0,x1,y1,bytes,hash
//!   <end ps>,<part name>,<first column>,<first row>,<last column>,<last row>,<bytes>,<hash>
//! traces/display-<part>.<format>   each pixel panel's frame memory at the run's end
//! traces/display-<part>.png        what its glass shows then
//! ```
//!
//! A row is one pixel write the panel closed: its window as the column and row commands set
//! it, both ends included, the bytes it took, and the first 16 hex digits of their SHA-256. A
//! frame memory file holds the panel's memory in its format: an `rgb565` file each pixel's two
//! bytes in the order the panel took them, rows top to bottom, each row left to right, and a
//! `mono-pages` file one bit a pixel in pages of 8 rows. The PNG is the glass, as [`glass`]
//! draws it.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use orbe_model::{Settings, Shows, Time, View};

use crate::name::Name;

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
    /// The controller's own settings, by name, such as an `st77916`'s `madctl` and `colmod`.
    #[serde(flatten)]
    pub settings: Settings,
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

/// What a device that shows `shows`, a screen, shows in `view`, as a reader sees its panel.
pub(crate) fn panel(shows: Shows, view: View<'_>) -> Panel {
    let (
        Shows::Screen {
            kind,
            width,
            height,
            format,
        },
        View::Screen { panel, .. },
    ) = (shows, view)
    else {
        panic!("only a device that shows a screen is a display");
    };
    Panel {
        kind,
        width,
        height,
        format,
        flags: Flags {
            display_on: panel.on,
            inverted: panel.inverted,
            reset: panel.reset,
            sleep: panel.asleep,
        },
        settings: panel.settings,
        registers: u64::from(panel.registers),
        brightness: panel.backlight.map(|duty| Brightness {
            duty: duty.duty,
            steps: 1u64
                .checked_shl(u32::from(duty.resolution))
                .filter(|_| duty.resolution < 32)
                .expect("a duty resolution is below 32"),
            hz: duty.hz,
        }),
    }
}

/// The frame memory a screen shows in `view`, rows top to bottom, each row left to right.
pub(crate) fn frame(view: View<'_>) -> &[u8] {
    let View::Screen { frame, .. } = view else {
        panic!("only a screen has a frame memory");
    };
    frame
}

/// What the glass of a display part shows of `frame` under `panel`: `panel.width` by
/// `panel.height` pixels of red, green, blue and alpha, a byte each, rows top to bottom and each
/// row left to right. Each kind draws its own: [`st77916::glass`](crate::device::st77916::glass)
/// and [`ssd1306::glass`](crate::device::ssd1306::glass).
///
/// Panics on a kind with no glass, or a frame that is not its kind's size.
pub fn glass(panel: &Panel, frame: &[u8]) -> Vec<u8> {
    match panel.kind {
        "st77916" => crate::device::st77916::glass(panel.flags, frame),
        "ssd1306" => crate::device::ssd1306::glass(panel, frame),
        "hd44780_i2c" => crate::device::hd44780_i2c::glass(panel, frame),
        "max7219" => crate::device::max7219::glass(panel, frame),
        kind => panic!("a {kind} has no glass"),
    }
}

/// Red, green, blue and alpha, a byte each.
const RGBA: usize = 4;

/// Whether a glass pixel reads lit: opaque, with its brightest channel at least half scale.
fn lit(pixel: &[u8]) -> bool {
    pixel[3] == 255 && pixel[..3].iter().any(|&channel| channel >= 128)
}

/// The glass as text, one line per row, each ended by `\n`: `#` for a lit pixel, `.` for a dark
/// one, and a space for a pixel off the glass, which is transparent. `pixels` is what [`glass`]
/// returns for a panel `width` pixels wide.
pub fn art(width: u16, pixels: &[u8]) -> String {
    let row = usize::from(width) * RGBA;
    let mut text = String::with_capacity(pixels.len() / RGBA + pixels.len() / row.max(1));
    for line in pixels.chunks(row.max(1)) {
        for pixel in line.chunks(RGBA) {
            text.push(match (pixel[3], lit(pixel)) {
                (0, _) => ' ',
                (_, true) => '#',
                (_, false) => '.',
            });
        }
        text.push('\n');
    }
    text
}

/// How many pixels of the glass read lit, as [`art`] marks them `#`.
pub fn lit_count(pixels: &[u8]) -> usize {
    pixels.chunks(RGBA).filter(|pixel| lit(pixel)).count()
}

/// `pixels`, `width` by `height` pixels as [`glass`] returns them, as a PNG file: 8 bits a
/// channel, red, green, blue and alpha, no filter, in stored deflate blocks, so the bytes depend
/// on the pixels alone.
pub fn png(width: u16, height: u16, pixels: &[u8]) -> Vec<u8> {
    let row = usize::from(width) * RGBA;
    assert_eq!(pixels.len(), row * usize::from(height), "RGBA pixels");
    let mut raw = Vec::with_capacity(pixels.len() + usize::from(height));
    for line in pixels.chunks(row.max(1)) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xffff).collect();
    for (k, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(k + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    if blocks.is_empty() {
        zlib.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&u32::from(width).to_be_bytes());
    header.extend_from_slice(&u32::from(height).to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (b"IHDR", &header[..]),
        (b"IDAT", &zlib[..]),
        (b"IEND", &[][..]),
    ] {
        file.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = file.len();
        file.extend_from_slice(kind);
        file.extend_from_slice(data);
        let crc = crc32(&file[start..]);
        file.extend_from_slice(&crc.to_be_bytes());
    }
    file
}

/// The CRC-32 a PNG chunk ends with, of `bytes`.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The Adler-32 a zlib stream ends with, of `bytes`.
fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
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

    /// Each part's frame memory file and its glass as a PNG, by their names under `traces/`.
    pub fn images(&self) -> impl Iterator<Item = (String, Vec<u8>)> {
        self.parts.iter().flat_map(|(name, panel, image)| {
            let shown = png(panel.width, panel.height, &glass(panel, image));
            [
                (image_file(name, panel.format), image.clone()),
                (format!("display-{}.png", name.as_str()), shown),
            ]
        })
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
                    image: format!("traces/{}", image_file(name, panel.format)),
                    sha256,
                };
                (name.as_str().to_string(), observed)
            })
            .collect()
    }
}

fn image_file(part: &Name, format: &str) -> String {
    format!("display-{}.{format}", part.as_str())
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
            settings: Settings::new(&["madctl", "colmod"], [0, 0x55]),
            registers: 3,
            brightness: None,
        }
    }

    #[test]
    fn a_row_is_its_window_bytes_and_hash() {
        let mut hash = orbe_model::Sha256::new();
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
            hash: hash.finish()[..8].try_into().unwrap(),
        }];
        let displays = Displays::new(
            vec![(Name::new("lcd1").unwrap(), panel(), b"abc".to_vec())],
            rows,
        );
        assert_eq!(
            String::from_utf8(displays.csv()).unwrap(),
            format!(
                "t_ps,part,x0,y0,x1,y1,bytes,hash\n500,lcd1,0,0,1,0,3,{}\n",
                crate::link::hash(b"abc")
            )
        );
    }

    /// A 128 by 64 `ssd1306` panel with the settings Adafruit SSD1306 leaves.
    fn oled() -> Panel {
        Panel {
            kind: "ssd1306",
            width: 128,
            height: 64,
            format: "mono-pages",
            flags: Flags {
                display_on: true,
                inverted: false,
                reset: false,
                sleep: false,
            },
            settings: Settings::new(
                &crate::device::ssd1306::model::SETTINGS,
                [0xcf, 0, 0, 0, 0, 63, 1, 1, 0x12, 0x14],
            ),
            registers: 3,
            brightness: None,
        }
    }

    #[test]
    fn each_display_writes_its_frame_memory_and_its_glass_as_a_png() {
        let mut frame = vec![0; 1024];
        frame[127] = 0b1;
        let displays = Displays::new(
            vec![(Name::new("oled").unwrap(), oled(), frame.clone())],
            Vec::new(),
        );
        let images: Vec<(String, Vec<u8>)> = displays.images().collect();
        assert_eq!(images[0], ("display-oled.mono-pages".to_string(), frame));
        assert_eq!(images[1].0, "display-oled.png");
        let png = &images[1].1;
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[16..24], &[0, 0, 0, 128, 0, 0, 0, 64]);
        assert_eq!(png.len(), 8 + 25 + 12 + 2 + 5 + 64 * (1 + 128 * 4) + 4 + 12);
        // The zlib stream's check: every filter byte and pixel byte summed as Adler-32 says.
        let pixels = glass(&oled(), &images[0].1);
        assert_eq!(lit_count(&pixels), 1);
        assert_eq!(&art(128, &pixels)[..4], "#...");
    }

    #[test]
    fn a_crc_and_an_adler_match_their_published_check_values() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
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
