//! Decoding and printing on both chips, against GNU objdump's text.

use sim_xtensa::{Config, Decoded, Esp32, Esp32s3};

use crate::common::{read_fixture, sha256_hex};

fn bytes_of(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn one<C: Config>(hex: &str, pc: u32) -> String {
    C::render(&C::decode(&bytes_of(hex), pc).unwrap())
}

const NOP: [u8; 3] = [0xf0, 0x20, 0x00];
const NOP_N: [u8; 2] = [0x3d, 0xf0];

/// The 8-byte slot the decode sweep gave objdump for a candidate: the candidate's format length
/// of its bytes, then `nop` and `nop.n` padding.
fn slot(word: u32, width: usize) -> Vec<u8> {
    let [b0, b1, mut b2, _] = word.to_le_bytes();
    if width == 2 && b0 & 0x8 == 0 {
        b2 = 0;
    }
    if b0 & 0x8 == 0 {
        [&[b0, b1, b2][..], &NOP_N, &NOP].concat()
    } else if b0 & 0xe == 0xe {
        [&[b0][..], &NOP, &NOP_N, &NOP_N].concat()
    } else {
        [&[b0, b1][..], &NOP_N, &NOP_N, &NOP_N].concat()
    }
}

fn sample_slot(chip: &str, hex: &str) -> Vec<u8> {
    let raw = bytes_of(hex);
    let word = |len: usize| {
        raw[..len]
            .iter()
            .rev()
            .fold(0u32, |w, b| (w << 8) | u32::from(*b))
    };
    match (chip, raw.len()) {
        (_, 2) => slot(word(2), 2),
        (_, 3) | ("esp32", 4..) => slot(word(3), 3),
        ("esp32", _) => [&raw[..1], &NOP, &NOP_N, &NOP_N].concat(),
        ("esp32s3", 4) => [&raw[..], &NOP_N, &NOP_N].concat(),
        _ => [&raw[..1], &NOP, &NOP_N, &NOP_N].concat(),
    }
}

/// Renders the committed sample on both configurations and compares the text's SHA-256 with
/// GNU objdump's. The text is one `addr:\tbody` line per row, the rows 8 bytes apart from
/// 0x40080000, with the body's whitespace collapsed to single spaces as the sweep stores it.
#[test]
fn sample_matches_gnu_objdump_on_both_chips() {
    let list = String::from_utf8(read_fixture("decode-sample.txt")).unwrap();
    let want = String::from_utf8(read_fixture("decode-sample-gnu.sha256")).unwrap();
    let mut text = String::new();
    let mut chips = [0usize; 2];
    for (index, line) in list.lines().enumerate() {
        let (chip, hex) = line.split_once(' ').unwrap();
        let bytes = sample_slot(chip, hex);
        assert_eq!(bytes.len(), 8, "{chip} {hex} slot");
        let pc = 0x4008_0000 + 8 * index as u32;
        let body = match chip {
            "esp32" => {
                chips[0] += 1;
                Esp32::render(&Esp32::decode(&bytes, pc).unwrap())
            }
            "esp32s3" => {
                chips[1] += 1;
                Esp32s3::render(&Esp32s3::decode(&bytes, pc).unwrap())
            }
            other => panic!("unknown chip {other}"),
        };
        let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
        text.push_str(&format!("{pc:08x}:\t{body}\n"));
    }
    assert_eq!(chips, [44, 14], "rows per chip");
    assert_eq!(sha256_hex(text.as_bytes()), want.trim());
}

#[test]
fn classic_stream_matches_gnu_objdump() {
    let stream: &[(&str, &str)] = &[
        ("360102", "entry\ta1, 0x100"),
        ("22a0ff", "movi\ta2, 255"),
        ("22a100", "movi\ta2, 0x100"),
        ("7cf2", "movi.n\ta2, -1"),
        ("22af01", "movi\ta2, -255"),
        ("22af00", "movi\ta2, 0xffffff00"),
        ("22c37f", "addi\ta2, a3, 127"),
        ("22c380", "addi\ta2, a3, -128"),
        ("22d301", "addmi\ta2, a3, 0x100"),
        ("22d3ff", "addmi\ta2, a3, 0xffffff00"),
        ("22d3fe", "addmi\ta2, a3, 0xfffffe00"),
        ("22233f", "l32i\ta2, a3, 252"),
        ("222340", "l32i\ta2, a3, 0x100"),
        ("2203ff", "l8ui\ta2, a3, 255"),
        ("26022f", "beqi\ta2, -1, 0x4008005c"),
        ("07622c", "bbci\ta2, 0, 0x4008005c"),
        ("f77229", "bbci\ta2, 31, 0x4008005c"),
        ("302004", "extui\ta2, a3, 0, 1"),
        ("302075", "extui\ta2, a3, 16, 8"),
        ("102301", "slli\ta2, a3, 31"),
        ("004000", "break\t0, 0"),
        ("f04f00", "break\t15, 15"),
        ("2df0", "break.n\t0"),
        ("2dff", "break.n\t15"),
        ("808040", "rotw\t-8"),
        ("708040", "rotw\t7"),
        ("206f00", "rsil\ta2, 15"),
        ("007f00", "waiti\t15"),
        ("f012ca", "float.s\tf1, a2, 15"),
        ("40238e", "f64iter\ta2, a3, a4, 0, 0"),
        ("4023ff", "f64cmph\ta2, a3, a4, 15"),
        ("3df0", "nop.n"),
        ("06ffff", "j\t0x4008005c"),
        ("1692ff", "beqz\ta2, 0x4008005c"),
        ("210000", "l32r\ta2, 0x40040064"),
        ("768201", "loop\ta2, 0x4008006a"),
        ("3df0", "nop.n"),
        ("3df0", "nop.n"),
        ("0e", ".byte\t0xe"),
        ("ff", ".byte\t0xff"),
        ("000000", "ill"),
    ];
    let image: Vec<u8> = stream.iter().flat_map(|(hex, _)| bytes_of(hex)).collect();
    let mut pc = 0x4008_0000u32;
    let mut off = 0usize;
    for (_, expect) in stream {
        let decoded = Esp32::decode(&image[off..], pc).unwrap();
        assert_eq!(Esp32::render(&decoded), *expect, "pc {pc:#x}");
        off += decoded.size() as usize;
        pc = pc.wrapping_add(u32::from(decoded.size()));
    }
    assert_eq!(off, image.len());
}

#[test]
fn lsi_holes_and_padding_opcodes() {
    assert_eq!(one::<Esp32>("0d10", 0), "lsi\tf0, a0, 0");
    assert_eq!(one::<Esp32>("0d11", 0), "lsi\tf0, a1, 4");
    assert_eq!(one::<Esp32>("0d21", 0), "lsi\tf0, a1, 4");
    assert_eq!(one::<Esp32>("cd12", 0), "lsi\tf12, a2, 8");
    assert_eq!(one::<Esp32>("0d1f", 0), "lsi\tf0, a15, 60");
    assert_eq!(one::<Esp32>("fd10", 0), "lsi\tf15, a0, 0");
    assert_eq!(one::<Esp32>("b30a00", 0), "lsi\tf11, a10, 0");
    assert_eq!(one::<Esp32>("b3fa0f", 0), "lsi\tf11, a10, 60");
    assert_eq!(one::<Esp32>("008000", 0), "any4\tb0, b0:b1:b2:b3");
    assert_eq!(
        one::<Esp32>("00a000", 0),
        "any8\tb0, b0:b1:b2:b3:b4:b5:b6:b7"
    );
    assert_eq!(one::<Esp32>("1e", 0), ".byte\t0x1e");
}

#[test]
fn short_slice_is_one_byte() {
    assert_eq!(one::<Esp32>("22", 0), ".byte\t0x22");
    assert_eq!(one::<Esp32s3>("22", 0), ".byte\t0x22");
    assert!(Esp32::decode(&[], 0).is_none());
}

#[test]
fn s3_undefined_zero_and_format32_slot() {
    assert_eq!(
        Esp32s3::decode(&bytes_of("000100"), 0),
        Some(Decoded::Undefined(0))
    );
    assert_eq!(one::<Esp32s3>("000100", 0), ".byte\t00");
    assert_eq!(
        one::<Esp32s3>("0e000000", 0),
        "ee.vmulas.s16.accx.ld.ip.qup\tq0, a0, 0, q0, q0, q0, q0"
    );
}
