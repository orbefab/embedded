//! The flash of a chip that boots from it: the regions a netlist places, and the app the
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

use crate::blob::{Blob, Sha256};
use crate::diagnostic::FirmwareRule;

pub const SIZE: u32 = 16 << 20;
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

/// A parsed flash. Only parse builds one, so the regions lie in the flash and no two share a
/// byte, and the app's segments each lie in one memory, with the entry in the first window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flash {
    /// Each a [`Role::Flash`](crate::blob::Role::Flash) blob.
    regions: Vec<Blob>,
    bytes: Arc<[u8]>,
    entry: u32,
    segments: Vec<Segment>,
}

/// A segment of the app: `len` bytes at bus address `addr`, from flash offset `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub addr: u32,
    pub offset: u32,
    pub len: u32,
}

impl Flash {
    /// Places `regions`, each a [`Role::Flash`](crate::blob::Role::Flash) blob, and finds the app.
    pub fn parse(regions: Vec<Blob>) -> Result<Flash, FirmwareRule> {
        let mut bytes = vec![0xff; SIZE as usize];
        let mut placed: Vec<Range<u64>> = Vec::new();
        for region in &regions {
            let start = u64::from(region.role.offset().expect("a flash region"));
            let span = start..start + region.bytes.len() as u64;
            if span.end > u64::from(SIZE)
                || placed
                    .iter()
                    .any(|p| p.start < span.end && span.start < p.end)
            {
                return Err(FirmwareRule::Region);
            }
            bytes[span.start as usize..span.end as usize].copy_from_slice(&region.bytes);
            placed.push(span);
        }
        let app = app_partition(&bytes)?;
        let (entry, segments) = app_image(&bytes, app)?;
        Ok(Flash {
            regions,
            bytes: bytes.into(),
            entry,
            segments,
        })
    }

    /// In netlist order.
    pub fn regions(&self) -> &[Blob] {
        &self.regions
    }

    /// The whole flash.
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    pub fn entry(&self) -> u32 {
        self.entry
    }

    /// In the image's order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
}

/// The flash the app partition the bootloader would pick spans.
fn app_partition(bytes: &[u8]) -> Result<Range<usize>, FirmwareRule> {
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
            _ => return Err(FirmwareRule::Image),
        };
        match (kind, subtype) {
            (APP, FACTORY) => factory = Some(span),
            (APP, OTA_0) => ota_0 = Some(span),
            (DATA, OTADATA) if bytes[span].iter().any(|&byte| byte != 0xff) => {
                return Err(FirmwareRule::Image);
            }
            _ => {}
        }
    }
    factory.or(ota_0).ok_or(FirmwareRule::Image)
}

/// The entry and segments of the app image at the start of `partition`.
fn app_image(bytes: &[u8], partition: Range<usize>) -> Result<(u32, Vec<Segment>), FirmwareRule> {
    let image = &bytes[partition.clone()];
    let word = |at: usize| {
        image
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(FirmwareRule::Image)
    };
    let header = image.get(..HEADER).ok_or(FirmwareRule::Image)?;
    let count = header[1];
    if header[0] != IMAGE_MAGIC
        || count > MAX_SEGMENTS
        || u16::from_le_bytes([header[12], header[13]]) != CHIP_ID_ESP32S3
    {
        return Err(FirmwareRule::Image);
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
            .ok_or(FirmwareRule::Image)?;
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
        return Err(FirmwareRule::Image);
    }
    if header[23] == 1 {
        let digest = image.get(end..end + 32).ok_or(FirmwareRule::Image)?;
        if Sha256::of(&image[..end]).bytes() != digest {
            return Err(FirmwareRule::Image);
        }
    }
    check_layout(&segments, entry)?;
    Ok((entry, segments))
}

/// Each segment lies in one memory, a mapped one at its flash offset modulo the MMU page, and the
/// entry in a segment on the instruction bus.
fn check_layout(segments: &[Segment], entry: u32) -> Result<(), FirmwareRule> {
    let inside = |range: &Range<u32>, segment: &Segment| {
        range.start <= segment.addr
            && u64::from(segment.addr) + u64::from(segment.len) <= u64::from(range.end)
    };
    for segment in segments {
        if MAPPED.iter().any(|range| inside(range, segment))
            && segment.addr % MMU_PAGE != segment.offset % MMU_PAGE
        {
            return Err(FirmwareRule::Image);
        }
        if !MAPPED
            .iter()
            .chain(&LOADED)
            .any(|range| inside(range, segment))
        {
            return Err(FirmwareRule::Unmapped);
        }
    }
    let code = &LOADED[0];
    let starts = segments.iter().any(|segment| {
        inside(code, segment)
            && segment.addr <= entry
            && u64::from(entry) < u64::from(segment.addr) + u64::from(segment.len)
    });
    if !starts {
        return Err(FirmwareRule::Unmapped);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::Role;

    fn blob(offset: u32, bytes: Vec<u8>) -> Blob {
        Blob {
            role: Role::Flash(offset),
            sha256: Sha256::of(&bytes),
            bytes: bytes.into(),
        }
    }

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
            let digest = *Sha256::of(&image).bytes();
            image.extend(digest);
        }
        image
    }

    fn flash(table: Vec<u8>, image: Vec<u8>) -> Result<Flash, FirmwareRule> {
        Flash::parse(vec![blob(0x8000, table), blob(0x10000, image)])
    }

    #[test]
    fn the_bootloader_picks_ota_0_and_its_segments() {
        let flash = flash(table(), image(true)).unwrap();
        assert_eq!(flash.entry(), 0x4037_5000);
        let segments = flash.segments();
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments[1],
            Segment {
                addr: 0x4200_002c,
                offset: 0x1002c,
                len: 4
            }
        );
        assert_eq!(flash.bytes()[0x1002c..0x10030], [5, 6, 7, 8]);
        assert_eq!(flash.bytes()[0xe000], 0xff, "a byte no region places");
    }

    #[test]
    fn a_bad_checksum_hash_or_otadata_is_not_booted() {
        let mut bad = image(false);
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(
            flash(table(), bad),
            Err(FirmwareRule::Image),
            "the checksum"
        );
        let mut bad = image(true);
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(flash(table(), bad), Err(FirmwareRule::Image), "the hash");
        let mut regions = flash(table(), image(true)).unwrap().regions;
        regions.push(blob(0xe000, vec![1; 32]));
        assert_eq!(
            Flash::parse(regions),
            Err(FirmwareRule::Image),
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
        assert_eq!(flash(table(), shifted), Err(FirmwareRule::Image));
    }

    /// A row whose offset plus size passes 2^32 is refused, as u32 sums are the same on a 32-bit
    /// build, where a `usize` sum would wrap to a small end.
    #[test]
    fn a_partition_whose_end_overflows_is_not_booted() {
        let mut table = row(APP, OTA_0, 0xffff_0000, 0x0002_0000);
        table.extend(row(APP, OTA_0, 0x10000, 0x10000));
        table.extend([0xff; 32]);
        assert_eq!(flash(table, image(true)), Err(FirmwareRule::Image));
        let mut table = row(DATA, OTADATA, 0xffff_0000, 0x0002_0000);
        table.extend([0xff; 32]);
        assert_eq!(
            flash(table, image(true)),
            Err(FirmwareRule::Image),
            "otadata"
        );
    }

    #[test]
    fn regions_lie_in_the_flash_and_apart() {
        let regions = |offset| vec![blob(0x8000, table()), blob(offset, image(true))];
        assert_eq!(
            Flash::parse(regions(SIZE - 16)),
            Err(FirmwareRule::Region),
            "past the end"
        );
        assert_eq!(
            Flash::parse(regions(0x8010)),
            Err(FirmwareRule::Region),
            "on the table"
        );
    }
}
