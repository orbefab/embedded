//! The app on a flash the chip boots from: the regions a netlist places, and the app the
//! second-stage bootloader would start there, as its entry and segments.
//!
//! The flash is 16 MB, and every byte no region places reads `0xFF`. The partition table is at
//! `0x8000`. Otadata that is not blank is refused, since the model does not read it. With
//! otadata absent or blank, the app is the factory partition, else `ota_0`. The app image is
//! ESP-IDF's: a header with the ESP32-S3's chip ID, its segments, a checksum, and the SHA-256 of
//! all of that when the header asks for one. A segment for the instruction or data cache is
//! mapped, from a flash offset that matches its address modulo the 64 KB MMU page, and one for
//! SRAM or RTC memory is loaded. The partition table's MD5 row is
//! not checked. IDF v5.5.5, `components/bootloader_support/include/esp_app_format.h`,
//! `components/esp_partition/include/esp_flash_partitions.h` and
//! `components/bootloader_support/src/bootloader_utility.c`.

use std::ops::Range;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::Segment;

const SIZE: u32 = 16 << 20;
const PARTITION_TABLE: usize = 0x8000;
const PARTITION_ROWS: usize = 0xc00 / 32;
const PARTITION_MAGIC: [u8; 2] = [0xaa, 0x50];
const APP: u8 = 0;
const DATA: u8 = 1;
const FACTORY: u8 = 0x00;
const OTA_0: u8 = 0x10;
const OTADATA: u8 = 0x00;
const IMAGE_MAGIC: u8 = 0xe9;
const CHIP_ID_ESP32S3: u16 = 9;
const HEADER: usize = 24;
const MAX_SEGMENTS: u8 = 16;
const CHECKSUM_SEED: u8 = 0xef;
const MMU_PAGE: u32 = 0x1_0000;

/// Where a segment of an app may lie: the cache windows it is mapped through, then the memories
/// it is loaded into.
const MAPPED: [Range<u32>; 2] = [0x4200_0000..0x4400_0000, 0x3c00_0000..0x3e00_0000];
const LOADED: [Range<u32>; 4] = [
    0x4037_0000..0x403e_0000,
    0x3fc8_8000..0x3fd0_0000,
    0x600f_e000..0x6010_0000,
    0x5000_0000..0x5000_2000,
];

/// Why a flash does not boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRule {
    /// A region past the flash's end, or one that shares a byte with another.
    Region,
    /// No app the bootloader would start: no partition for it, otadata that is not blank, or an
    /// image whose header, checksum, hash or mapping is wrong.
    Image,
    /// A segment outside every memory, or an entry in no segment on the instruction bus.
    Unmapped,
}

/// The flash the regions make, and the app on it. Only [`App::parse`] builds one, so the
/// segments each lie in one memory, with the entry in the first window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// The whole flash.
    pub flash: Arc<[u8]>,
    pub entry: u32,
    /// In the image's order.
    pub segments: Vec<Segment>,
}

impl App {
    /// Places `regions`, each a flash offset and its bytes, and finds the app.
    pub fn parse(regions: &[(u32, &[u8])]) -> Result<App, ImageRule> {
        let mut bytes = vec![0xff; SIZE as usize];
        let mut placed: Vec<Range<u64>> = Vec::new();
        for &(offset, region) in regions {
            let start = u64::from(offset);
            let span = start..start + region.len() as u64;
            if span.end > u64::from(SIZE)
                || placed
                    .iter()
                    .any(|p| p.start < span.end && span.start < p.end)
            {
                return Err(ImageRule::Region);
            }
            bytes[span.start as usize..span.end as usize].copy_from_slice(region);
            placed.push(span);
        }
        let app = app_partition(&bytes)?;
        let (entry, segments) = app_image(&bytes, app)?;
        Ok(App {
            flash: bytes.into(),
            entry,
            segments,
        })
    }
}

/// The flash the app partition the bootloader would pick spans.
fn app_partition(bytes: &[u8]) -> Result<Range<usize>, ImageRule> {
    let mut factory = None;
    let mut ota_0 = None;
    for row in bytes[PARTITION_TABLE..][..32 * PARTITION_ROWS].chunks(32) {
        if row[..2] != PARTITION_MAGIC {
            break;
        }
        let (kind, subtype) = (row[2], row[3]);
        let offset = u32::from_le_bytes([row[4], row[5], row[6], row[7]]);
        let size = u32::from_le_bytes([row[8], row[9], row[10], row[11]]);
        // In u32, so a 32-bit build adds as a 64-bit one does.
        let span = match offset.checked_add(size) {
            Some(end) if end <= SIZE => offset as usize..end as usize,
            _ => return Err(ImageRule::Image),
        };
        match (kind, subtype) {
            (APP, FACTORY) => factory = Some(span),
            (APP, OTA_0) => ota_0 = Some(span),
            (DATA, OTADATA) if bytes[span].iter().any(|&byte| byte != 0xff) => {
                return Err(ImageRule::Image);
            }
            _ => {}
        }
    }
    factory.or(ota_0).ok_or(ImageRule::Image)
}

/// The entry and segments of the app image at the start of `partition`.
fn app_image(bytes: &[u8], partition: Range<usize>) -> Result<(u32, Vec<Segment>), ImageRule> {
    let image = &bytes[partition.clone()];
    let word = |at: usize| {
        image
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(ImageRule::Image)
    };
    let header = image.get(..HEADER).ok_or(ImageRule::Image)?;
    let count = header[1];
    if header[0] != IMAGE_MAGIC
        || count > MAX_SEGMENTS
        || u16::from_le_bytes([header[12], header[13]]) != CHIP_ID_ESP32S3
    {
        return Err(ImageRule::Image);
    }
    let entry = word(4)?;
    let mut segments = Vec::new();
    let mut checksum = CHECKSUM_SEED;
    let mut at = HEADER;
    for _ in 0..count {
        let (addr, len) = (word(at)?, word(at + 4)?);
        let data = image
            .get(at + 8..)
            .and_then(|rest| rest.get(..len as usize))
            .ok_or(ImageRule::Image)?;
        checksum = data.iter().fold(checksum, |sum, &byte| sum ^ byte);
        segments.push(Segment {
            addr,
            offset: (partition.start + at + 8) as u32,
            len,
        });
        at += 8 + len as usize;
    }
    let end = (at + 1).next_multiple_of(16);
    if image.get(end - 1) != Some(&checksum) {
        return Err(ImageRule::Image);
    }
    if header[23] == 1 {
        let digest = image.get(end..end + 32).ok_or(ImageRule::Image)?;
        if Sha256::digest(&image[..end])[..] != *digest {
            return Err(ImageRule::Image);
        }
    }
    check_layout(&segments, entry)?;
    Ok((entry, segments))
}

/// Each segment lies in one memory, a mapped one at its flash offset modulo the MMU page, and the
/// entry in a segment on the instruction bus.
fn check_layout(segments: &[Segment], entry: u32) -> Result<(), ImageRule> {
    let inside = |range: &Range<u32>, segment: &Segment| {
        range.start <= segment.addr
            && u64::from(segment.addr) + u64::from(segment.len) <= u64::from(range.end)
    };
    for segment in segments {
        if MAPPED.iter().any(|range| inside(range, segment))
            && segment.addr % MMU_PAGE != segment.offset % MMU_PAGE
        {
            return Err(ImageRule::Image);
        }
        if !MAPPED
            .iter()
            .chain(&LOADED)
            .any(|range| inside(range, segment))
        {
            return Err(ImageRule::Unmapped);
        }
    }
    let code = &LOADED[0];
    let starts = segments.iter().any(|segment| {
        inside(code, segment)
            && segment.addr <= entry
            && u64::from(entry) < u64::from(segment.addr) + u64::from(segment.len)
    });
    if !starts {
        return Err(ImageRule::Unmapped);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: u8, subtype: u8, offset: u32, size: u32) -> Vec<u8> {
        let mut row = vec![0xaa, 0x50, kind, subtype];
        row.extend(offset.to_le_bytes());
        row.extend(size.to_le_bytes());
        row.extend([0; 20]);
        row
    }

    /// A table with otadata and `ota_0` at `0x10000`.
    fn table() -> Vec<u8> {
        let mut table = row(DATA, OTADATA, 0xe000, 0x2000);
        table.extend(row(APP, OTA_0, 0x10000, 0x10000));
        table.extend([0xff; 32]);
        table
    }

    /// An image with one IRAM segment holding the entry, and one IROM segment.
    fn image(sha: bool) -> Vec<u8> {
        let mut image = vec![IMAGE_MAGIC, 2, 2, 0x2f];
        image.extend(0x4037_5000u32.to_le_bytes());
        image.extend([0xee, 0, 0, 0]);
        image.extend(CHIP_ID_ESP32S3.to_le_bytes());
        image.extend([0; 9]);
        image.push(u8::from(sha));
        let mut checksum = CHECKSUM_SEED;
        for (addr, data) in [
            (0x4037_5000u32, [1u8, 2, 3, 4]),
            (0x4200_002c, [5, 6, 7, 8]),
        ] {
            image.extend(addr.to_le_bytes());
            image.extend(4u32.to_le_bytes());
            image.extend(data);
            checksum = data.iter().fold(checksum, |sum, &byte| sum ^ byte);
        }
        image.resize((image.len() + 1).next_multiple_of(16) - 1, 0);
        image.push(checksum);
        if sha {
            let digest = Sha256::digest(&image);
            image.extend(digest);
        }
        image
    }

    fn flash(table: Vec<u8>, image: Vec<u8>) -> Result<App, ImageRule> {
        App::parse(&[(0x8000, &table), (0x10000, &image)])
    }

    #[test]
    fn the_bootloader_picks_ota_0_and_its_segments() {
        let app = flash(table(), image(true)).unwrap();
        assert_eq!(app.entry, 0x4037_5000);
        let segments = &app.segments;
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments[1],
            Segment {
                addr: 0x4200_002c,
                offset: 0x1002c,
                len: 4
            }
        );
        assert_eq!(app.flash[0x1002c..0x10030], [5, 6, 7, 8]);
        assert_eq!(app.flash[0xe000], 0xff, "a byte no region places");
    }

    #[test]
    fn a_bad_checksum_hash_or_otadata_is_not_booted() {
        let mut bad = image(false);
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(flash(table(), bad), Err(ImageRule::Image), "the checksum");
        let mut bad = image(true);
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(flash(table(), bad), Err(ImageRule::Image), "the hash");
        let (table, image, otadata) = (table(), image(true), [1; 32]);
        assert_eq!(
            App::parse(&[(0x8000, &table), (0x10000, &image), (0xe000, &otadata)]),
            Err(ImageRule::Image),
            "a set otadata"
        );
    }

    /// IDF's bootloader refuses a mapped segment whose address and flash offset differ modulo
    /// the 64 KB page, as the MMU maps whole pages.
    #[test]
    fn a_mapped_segment_off_its_page_is_not_booted() {
        let mut shifted = image(false);
        let at = HEADER + 12;
        shifted[at..at + 4].copy_from_slice(&0x4200_0030u32.to_le_bytes());
        assert_eq!(flash(table(), shifted), Err(ImageRule::Image));
    }

    /// A row whose offset plus size passes 2^32 is refused, as u32 sums are the same on a 32-bit
    /// build, where a `usize` sum would wrap to a small end.
    #[test]
    fn a_partition_whose_end_overflows_is_not_booted() {
        let mut table = row(APP, OTA_0, 0xffff_0000, 0x0002_0000);
        table.extend(row(APP, OTA_0, 0x10000, 0x10000));
        table.extend([0xff; 32]);
        assert_eq!(flash(table, image(true)), Err(ImageRule::Image));
        let mut table = row(DATA, OTADATA, 0xffff_0000, 0x0002_0000);
        table.extend([0xff; 32]);
        assert_eq!(flash(table, image(true)), Err(ImageRule::Image), "otadata");
    }

    #[test]
    fn regions_lie_in_the_flash_and_apart() {
        let (table, image) = (table(), image(true));
        let regions = |offset| App::parse(&[(0x8000, &table), (offset, &image)]);
        assert_eq!(regions(SIZE - 16), Err(ImageRule::Region), "past the end");
        assert_eq!(regions(0x8010), Err(ImageRule::Region), "on the table");
    }
}
