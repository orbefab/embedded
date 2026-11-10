//! A PNG writer for 8-bit RGBA. The image data sits in stored deflate blocks, uncompressed, so it
//! needs no dependency and the same pixels always give the same bytes.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
/// The most bytes a stored deflate block holds.
const BLOCK: usize = 0xffff;

/// The PNG of `pixels`, `width` by `height` pixels of red, green, blue and alpha, a byte each,
/// rows top to bottom. Panics when either side is 0 or `pixels` is not `width * height * 4`
/// bytes.
pub fn rgba(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    assert!(width > 0 && height > 0, "a PNG has pixels");
    assert_eq!(pixels.len(), row * height as usize, "four bytes a pixel");
    // Each row starts with filter type 0, the row as it is.
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in pixels.chunks_exact(row) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // Bit depth 8, colour type 6 (RGBA), deflate, adaptive filtering, no interlace.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = SIGNATURE.to_vec();
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

/// `data`, at least one byte, as a zlib stream of stored blocks.
fn zlib(data: &[u8]) -> Vec<u8> {
    // Deflate with a 32 KiB window, no dictionary, and a header check that makes 0x7801 a
    // multiple of 31.
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(BLOCK).peekable();
    while let Some(block) = blocks.next() {
        let last = u8::from(blocks.peek().is_none());
        let len = u16::try_from(block.len()).expect("a stored block fits in 16 bits");
        out.push(last);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).expect("a chunk fits in 32 bits");
    png.extend_from_slice(&len.to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

/// The CRC-32 of ISO 3309, the one PNG names, bit by bit.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                crc >> 1 ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksums_match_their_published_check_values() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    /// A one-pixel image, every byte known: the signature, then IHDR, IDAT and IEND, each with
    /// its CRC. The IDAT holds one stored block of the filter byte and the pixel.
    #[test]
    fn one_red_pixel_is_these_bytes() {
        let png = rgba(1, 1, &[255, 0, 0, 255]);
        let mut want = SIGNATURE.to_vec();
        want.extend_from_slice(b"\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0");
        want.extend_from_slice(&crc32(b"IHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0").to_be_bytes());
        let idat = b"IDAT\x78\x01\x01\x05\x00\xfa\xff\x00\xff\x00\x00\xff\x05\x00\x01\xff";
        want.extend_from_slice(b"\0\0\0\x10");
        want.extend_from_slice(idat);
        want.extend_from_slice(&crc32(idat).to_be_bytes());
        want.extend_from_slice(b"\0\0\0\0IEND\xae\x42\x60\x82");
        assert_eq!(png, want);
    }

    /// 70000 bytes of image data take two stored blocks, 65535 bytes then the rest, and only
    /// the second is marked last.
    #[test]
    fn data_past_one_block_splits_and_marks_only_the_last() {
        let data = vec![7; 70000];
        let stream = zlib(&data);
        assert_eq!(stream[2..7], [0, 0xff, 0xff, 0, 0]);
        let second = 7 + BLOCK;
        let rest = u16::try_from(70000 - BLOCK).unwrap();
        assert_eq!(stream[second], 1);
        assert_eq!(stream[second + 1..second + 3], rest.to_le_bytes());
        assert_eq!(stream[second + 3..second + 5], (!rest).to_le_bytes());
        assert_eq!(stream.len(), 2 + 5 + BLOCK + 5 + usize::from(rest) + 4);
    }
}
