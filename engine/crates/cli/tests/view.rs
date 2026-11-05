//! `sim view` on `examples/st77916-replay`: one PNG for its panel, whose pixels are what
//! `st77916::glass` draws from the run folder, whose bytes are the same on every run, and which
//! leaves the run folder as `sim run` wrote it.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sim_circuit::st77916;

use common::{files, repo};

fn sim(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sim"))
        .args(args)
        .output()
        .expect("spawn sim")
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sim-cli-view-{name}-{}", std::process::id()))
}

fn stdout(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The pixels of a PNG `sim view` writes: one IDAT of stored deflate blocks, each row behind
/// filter type 0. Checks each chunk's CRC and the stream's Adler-32 on the way.
fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(
        png[..8],
        [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
    );
    let mut chunks = Vec::new();
    let mut at = 8;
    while at < png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let body = &png[at + 4..at + 8 + len];
        let crc = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
        assert_eq!(crc32(body), crc, "the CRC of {:?}", &body[..4]);
        chunks.push((&body[..4], &body[4..]));
        at += 12 + len;
    }
    let kinds: Vec<&[u8]> = chunks.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(kinds, [&b"IHDR"[..], b"IDAT", b"IEND"]);
    let header = chunks[0].1;
    let width = u32::from_be_bytes(header[..4].try_into().unwrap());
    let height = u32::from_be_bytes(header[4..8].try_into().unwrap());
    assert_eq!(header[8..], [8, 6, 0, 0, 0], "8-bit RGBA, no interlace");
    let stream = chunks[1].1;
    assert_eq!(stream[..2], [0x78, 0x01]);
    let mut raw = Vec::new();
    let mut at = 2;
    loop {
        let last = stream[at];
        let len = u16::from_le_bytes([stream[at + 1], stream[at + 2]]);
        assert_eq!(u16::from_le_bytes([stream[at + 3], stream[at + 4]]), !len);
        raw.extend_from_slice(&stream[at + 5..at + 5 + usize::from(len)]);
        at += 5 + usize::from(len);
        if last == 1 {
            break;
        }
        assert_eq!(last, 0, "a stored block");
    }
    assert_eq!(stream[at..], adler32(&raw).to_be_bytes());
    let row = width as usize * 4;
    let mut pixels = Vec::new();
    for line in raw.chunks(row + 1) {
        assert_eq!(line[0], 0, "filter type 0");
        pixels.extend_from_slice(&line[1..]);
    }
    assert_eq!(pixels.len(), row * height as usize);
    (width, height, pixels)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (a, b) = data.iter().fold((1u32, 0u32), |(a, b), &byte| {
        let a = (a + u32::from(byte)) % 65521;
        (a, (b + a) % 65521)
    });
    b << 16 | a
}

#[test]
fn the_replay_panel_is_one_png_of_its_glass_the_same_on_every_run() {
    let dir = scratch("replay");
    let _ = fs::remove_dir_all(&dir);
    let run = dir.join("run");
    let netlist = repo("examples/st77916-replay/netlist.json");
    let ran = sim(&[Path::new("run"), &netlist, Path::new("--out"), &run]);
    assert!(ran.status.success(), "{ran:?}");
    let before = files(&run);

    let (first, second) = (dir.join("first"), dir.join("second"));
    for out in [&first, &second] {
        let viewed = sim(&[Path::new("view"), &run, Path::new("--out"), out]);
        assert!(viewed.status.success(), "{viewed:?}");
        let png = out.join("display-lcd1.png").display().to_string();
        let want = serde_json::json!({
            "status": "ok",
            "out": out.display().to_string(),
            "images": { "lcd1": png },
        });
        assert_eq!(stdout(&viewed), want);
    }
    assert_eq!(files(&run), before, "the run folder is as sim run wrote it");
    let png = fs::read(first.join("display-lcd1.png")).unwrap();
    assert_eq!(files(&first), files(&second), "two runs, the same bytes");
    assert_eq!(files(&first).len(), 1);

    let observations: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("observations.json")).unwrap()).unwrap();
    let entry = &observations["displays"]["lcd1"];
    let flags = serde_json::from_value(entry["flags"].clone()).unwrap();
    let frame = fs::read(run.join(entry["image"].as_str().unwrap())).unwrap();
    let (width, height, pixels) = decode(&png);
    assert_eq!((width, height), (360, 360));
    assert!(
        pixels == st77916::glass(flags, &frame),
        "the PNG is the glass"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_run_with_no_display_part_writes_no_image() {
    let dir = scratch("none");
    let _ = fs::remove_dir_all(&dir);
    let run = dir.join("run");
    let netlist = repo("examples/rc-switch/netlist.json");
    assert!(
        sim(&[Path::new("run"), &netlist, Path::new("--out"), &run])
            .status
            .success()
    );
    let out = dir.join("out");
    let viewed = sim(&[Path::new("view"), &run, Path::new("--out"), &out]);
    assert!(viewed.status.success(), "{viewed:?}");
    assert_eq!(stdout(&viewed)["images"], serde_json::json!({}));
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn view_without_out_is_a_usage_error_and_writes_nothing() {
    let dir = scratch("no-out");
    let _ = fs::remove_dir_all(&dir);
    let run = dir.join("run");
    let netlist = repo("examples/st77916-replay/netlist.json");
    assert!(
        sim(&[Path::new("run"), &netlist, Path::new("--out"), &run])
            .status
            .success()
    );
    let cwd = dir.join("cwd");
    fs::create_dir(&cwd).unwrap();
    let viewed = Command::new(env!("CARGO_BIN_EXE_sim"))
        .current_dir(&cwd)
        .args([Path::new("view"), run.as_path()])
        .output()
        .expect("spawn sim");
    assert_eq!(viewed.status.code(), Some(2));
    assert!(viewed.stderr.is_empty(), "{viewed:?}");
    assert_eq!(stdout(&viewed)["status"], "usage_error");
    assert!(fs::read_dir(&cwd).unwrap().next().is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_image_path_other_than_the_part_frame_is_an_io_error() {
    let dir = scratch("image-path");
    let _ = fs::remove_dir_all(&dir);
    let run = dir.join("run");
    let netlist = repo("examples/st77916-replay/netlist.json");
    assert!(
        sim(&[Path::new("run"), &netlist, Path::new("--out"), &run])
            .status
            .success()
    );
    let observations_path = run.join("observations.json");
    let mut observations: serde_json::Value =
        serde_json::from_slice(&fs::read(&observations_path).unwrap()).unwrap();
    observations["displays"]["lcd1"]["image"] = serde_json::json!("traces/display-other.rgb565");
    fs::write(
        &observations_path,
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
    let out = dir.join("out");
    let viewed = sim(&[Path::new("view"), &run, Path::new("--out"), &out]);
    assert_eq!(viewed.status.code(), Some(3));
    let body = stdout(&viewed);
    assert_eq!(body["status"], "io_error");
    assert_eq!(body["path"], observations_path.display().to_string());
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("traces/display-lcd1.rgb565"));
    assert!(!message.contains("display-other"));
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}

/// A folder with no `observations.json`, and a frame memory file that is not the one its entry
/// names, are `io_error`, exit 3, and write no image.
#[test]
fn an_unreadable_run_folder_is_an_io_error_and_writes_nothing() {
    let dir = scratch("broken");
    let _ = fs::remove_dir_all(&dir);
    let out = dir.join("out");
    let missing = sim(&[
        Path::new("view"),
        &dir.join("absent"),
        Path::new("--out"),
        &out,
    ]);
    assert_eq!(missing.status.code(), Some(3));
    assert_eq!(stdout(&missing)["status"], "io_error");

    let run = dir.join("run");
    let netlist = repo("examples/st77916-replay/netlist.json");
    assert!(
        sim(&[Path::new("run"), &netlist, Path::new("--out"), &run])
            .status
            .success()
    );
    let image = run.join("traces/display-lcd1.rgb565");
    let mut frame = fs::read(&image).unwrap();
    frame[0] ^= 1;
    fs::write(&image, frame).unwrap();
    let stale = sim(&[Path::new("view"), &run, Path::new("--out"), &out]);
    assert_eq!(stale.status.code(), Some(3));
    let body = stdout(&stale);
    assert_eq!(body["status"], "io_error");
    assert_eq!(body["path"], image.display().to_string());
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}
