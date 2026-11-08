//! The dual-core probe on both cores: the PRO CPU starts the APP CPU through the ROM, the two
//! trade a mailbox and share an `s32c1i` spinlock. The probe prints what the board printed, and
//! a run split at any cycles prints it at the same ticks.

use std::path::PathBuf;

use orbe_espressif::s3::{Esp32s3, Halt, Tick};
use sha2::{Digest, Sha256};

/// The pinned S3 revision 0 ROM ELF, as `packages/engine/scripts/blobs.sh` names it.
const ROM_SHA256: &str = "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd";
/// Ticks per crystal cycle, the CPU clock the probe runs on.
const CYCLE: u64 = 12;
/// Past the probe's end, which comes at about 0.2 s.
const END: Tick = Tick(480_000_000);

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The ROM from the folder `packages/engine/scripts/blobs.sh` fills: `$ORBE_BLOBS`, else
/// `$XDG_CACHE_HOME/orbe/blobs`, else `~/.cache/orbe/blobs`.
fn rom() -> Vec<u8> {
    let set = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    let folder = set("ORBE_BLOBS").map(PathBuf::from).unwrap_or_else(|| {
        set("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cache")
            })
            .join("orbe/blobs")
    });
    let path = folder.join(ROM_SHA256);
    let rom = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "the ROM blob {ROM_SHA256} is not in {}; run packages/engine/scripts/blobs.sh",
            folder.display()
        )
    });
    assert_eq!(sha256(&rom), ROM_SHA256, "{}", path.display());
    rom
}

/// The probe's entry and its loadable segments, each as an address and its bytes with the
/// zeroed tail.
fn segments(elf: &[u8]) -> (u32, Vec<(u32, Vec<u8>)>) {
    let u16_at = |at: usize| u16::from_le_bytes([elf[at], elf[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(elf[at..at + 4].try_into().unwrap());
    let (entry, phoff) = (u32_at(24), u32_at(28) as usize);
    let (size, count) = (usize::from(u16_at(42)), usize::from(u16_at(44)));
    let loads = (0..count)
        .map(|k| phoff + k * size)
        .filter(|&header| u32_at(header) == 1)
        .map(|header| {
            let (offset, addr) = (u32_at(header + 4) as usize, u32_at(header + 8));
            let (file, memory) = (u32_at(header + 16) as usize, u32_at(header + 20) as usize);
            let mut bytes = elf[offset..offset + file].to_vec();
            bytes.resize(memory, 0);
            (addr, bytes)
        })
        .collect();
    (entry, loads)
}

/// The probe at power-on, with `g` on its console so it starts.
fn probe() -> Esp32s3 {
    let elf = fixture("s3-dualcore.elf");
    let board = String::from_utf8(fixture("s3-dualcore.txt")).unwrap();
    let want = board
        .lines()
        .find_map(|line| line.strip_prefix("image_sha256: "));
    assert_eq!(want, Some(sha256(&elf).as_str()), "the image the board ran");
    let (entry, loads) = segments(&elf);
    let loads = loads.iter().map(|(addr, bytes)| (*addr, bytes.as_slice()));
    let mut chip = Esp32s3::new(&[], entry, loads, Some(&rom()));
    chip.receive(Tick(0), b"g");
    chip
}

/// What a run shows outside: each console byte with its tick, each halt, and the final tick.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    console: Vec<(Tick, u8)>,
    halts: Vec<Halt>,
    end: Tick,
}

impl Seen {
    fn text(&self) -> String {
        let bytes: Vec<u8> = self.console.iter().map(|&(_, byte)| byte).collect();
        String::from_utf8(bytes).unwrap().replace('\r', "")
    }

    /// The tick of the console byte that ends the first line starting with `start`.
    fn line_end(&self, start: &str) -> Tick {
        let mut line = Vec::new();
        for &(at, byte) in &self.console {
            match byte {
                b'\n' if line.starts_with(start.as_bytes()) => return at,
                b'\n' => line.clear(),
                b'\r' => {}
                byte => line.push(byte),
            }
        }
        panic!("no line starts with {start}");
    }
}

/// Runs `chip` to each of `limits` in turn, then to [`END`].
fn run(mut chip: Esp32s3, limits: impl IntoIterator<Item = Tick>) -> Seen {
    let mut seen = Seen {
        console: Vec::new(),
        halts: Vec::new(),
        end: Tick(0),
    };
    for limit in limits.into_iter().chain([END]) {
        loop {
            match chip.run_until(limit) {
                Some(halt) => seen.halts.push(halt),
                None => {
                    assert!(chip.tick() > limit, "tick() passes the limit {limit:?}");
                    break;
                }
            }
        }
        while let Some(byte) = chip.take_sent(limit) {
            seen.console.push(byte);
        }
    }
    seen.end = chip.tick();
    seen
}

#[test]
fn the_dualcore_probe_prints_its_board_capture() {
    let seen = run(probe(), []);
    let text = seen.text();
    let board = String::from_utf8(fixture("s3-dualcore.txt")).unwrap();
    let want = &board[board.find("BEGIN dualcore").unwrap()..];
    let begin = text.find("BEGIN dualcore").expect("the probe began");
    let end = text.find("END dualcore\n").expect("the probe ended") + "END dualcore\n".len();
    assert_eq!(&text[begin..end], want);
}

/// A step from `x`, from 1 to 4096 cycles.
fn xorshift(x: &mut u64) -> u64 {
    *x ^= *x << 13;
    *x ^= *x >> 7;
    *x ^= *x << 17;
    *x % 4096 + 1
}

/// Three runs split at random steps, and inside the window from 1000 cycles before the APP
/// CPU's release to the end of the second mailbox round at steps of 1, 2, 3 and 7 cycles in
/// turn, show what one run shows.
#[test]
fn the_dualcore_probe_runs_the_same_however_the_run_splits() {
    let whole = run(probe(), []);
    let window = whole.line_end("core1_ctrl_before").0 - 1000 * CYCLE..whole.line_end("mbox=02").0;
    for seed in [1u64, 2, 3] {
        let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut fine = [1, 2, 3, 7].into_iter().cycle();
        let mut limits = Vec::new();
        let mut at = 0;
        loop {
            let cycles = if window.contains(&at) {
                fine.next().unwrap()
            } else {
                xorshift(&mut x)
            };
            at += cycles * CYCLE;
            if at >= END.0 {
                break;
            }
            limits.push(Tick(at));
        }
        let split = run(probe(), limits);
        assert_eq!(split.halts, whole.halts, "seed {seed}");
        assert_eq!(split.end, whole.end, "seed {seed}");
        assert!(split.console == whole.console, "seed {seed}");
    }
}
