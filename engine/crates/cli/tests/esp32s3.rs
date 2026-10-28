//! `esp32s3` parts through `sim run`: where the firmware blob is found, the `netlist.blob` error
//! when it is not, and the diagnostics a program raises when it leaves a watchdog running,
//! touches a register the model does not keep, reaches an instruction the core does not run, or
//! jumps out of its memory, and an FPU op the core now runs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::repo;
use sim_circuit::{Refused, Session};
use sim_core::blob::{Blobs, Sha256};
use sim_core::time::Time;

fn sim(args: &[&str]) -> (i32, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_sim"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let doc = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.code().unwrap(), doc)
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("sim-cli-esp32s3-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn example() -> serde_json::Value {
    serde_json::from_slice(&fs::read(repo("examples/esp32s3-blink/netlist.json")).unwrap()).unwrap()
}

fn example_blobs() -> PathBuf {
    repo("examples/esp32s3-blink/blobs")
}

/// Writes `netlist` into `dir` and runs it, with `--blobs` for each of `blobs`.
fn run(dir: &Path, netlist: &serde_json::Value, blobs: &[&Path]) -> (i32, serde_json::Value) {
    let path = dir.join("netlist.json");
    fs::write(&path, serde_json::to_vec_pretty(netlist).unwrap()).unwrap();
    let out = dir.join("out");
    let mut args = vec![
        "run".to_string(),
        path.display().to_string(),
        "--out".to_string(),
        out.display().to_string(),
    ];
    for blob in blobs {
        args.extend(["--blobs".to_string(), blob.display().to_string()]);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    sim(&args)
}

fn diagnostics(doc: &serde_json::Value) -> Vec<serde_json::Value> {
    doc["diagnostics"].as_array().unwrap().clone()
}

fn firmware(netlist: &mut serde_json::Value, sha256: &str, bytes: usize) {
    netlist["parts"][1]["firmware"]["elf"] =
        serde_json::json!({ "sha256": sha256, "bytes": bytes });
}

#[test]
fn a_netlist_with_no_blobs_folder_names_the_missing_blob_and_blobs_finds_it() {
    let dir = scratch("missing");
    let netlist = example();
    let (code, doc) = run(&dir, &netlist, &[]);
    assert_eq!((code, doc["status"].as_str()), (1, Some("failed")));
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    let sha256 = netlist["parts"][1]["firmware"]["elf"]["sha256"]
        .as_str()
        .unwrap();
    assert_eq!(found[0]["code"], "netlist.blob");
    assert_eq!(found[0]["refs"], serde_json::json!(["#u1"]));
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "bytes": 3800, "found": null, "role": "elf", "rule": "missing", "sha256": sha256 })
    );
    assert!(found[0]["message"].as_str().unwrap().contains("--blobs"));

    let (code, doc) = run(&dir, &netlist, &[&example_blobs()]);
    assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("out/manifest.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["blobs"],
        serde_json::json!([{ "role": "elf", "sha256": sha256, "bytes": 3800 }])
    );
}

#[test]
fn a_blob_that_is_not_the_one_named_is_refused() {
    let dir = scratch("mismatch");
    let mut netlist = example();
    let sha256 = netlist["parts"][1]["firmware"]["elf"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let other = dir.join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join(&sha256), b"other bytes").unwrap();
    let (code, doc) = run(&dir, &netlist, &[&other]);
    assert_eq!(code, 1);
    let found = diagnostics(&doc);
    assert_eq!(
        (
            found.len(),
            &found[0]["data"]["rule"],
            &found[0]["data"]["found"]
        ),
        (1, &serde_json::json!("hash"), &serde_json::json!(11))
    );
    assert!(found[0]["message"].as_str().unwrap().contains("--blobs"));

    firmware(&mut netlist, &sha256, 3801);
    let (code, doc) = run(&dir, &netlist, &[&example_blobs()]);
    assert_eq!(code, 1);
    let found = diagnostics(&doc);
    assert_eq!(
        (
            found.len(),
            &found[0]["data"]["rule"],
            &found[0]["data"]["found"]
        ),
        (1, &serde_json::json!("length"), &serde_json::json!(3800))
    );
    assert!(found[0]["message"].as_str().unwrap().contains("--blobs"));
}

#[test]
fn a_blob_list_names_no_folder() {
    let mut netlist = example();
    let sha256 = netlist["parts"][1]["firmware"]["elf"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let message = |netlist: &serde_json::Value, blobs: &Blobs| {
        let json = serde_json::to_vec(netlist).unwrap();
        let Err(Refused::Invalid(found)) = Session::open_document_with(&json, blobs) else {
            panic!("the netlist is invalid");
        };
        serde_json::to_value(&found[0]).unwrap()["message"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        message(&netlist, &Blobs::new()),
        format!("#u1 elf blob {sha256} is not in the blob list")
    );
    let mut blobs = Blobs::new();
    blobs.add(fs::read(example_blobs().join(&sha256)).unwrap().into());
    firmware(&mut netlist, &sha256, 3801);
    assert_eq!(
        message(&netlist, &blobs),
        format!("#u1 elf blob {sha256} in the blob list has 3800 bytes; the netlist says 3801")
    );
}

#[test]
fn the_first_good_copy_wins_and_a_file_the_netlist_does_not_name_is_not_read() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("search");
    let netlist = example();
    let sha256 = netlist["parts"][1]["firmware"]["elf"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    fs::create_dir_all(dir.join("blobs")).unwrap();
    fs::copy(
        example_blobs().join(&sha256),
        dir.join("blobs").join(&sha256),
    )
    .unwrap();
    let bad = dir.join("bad");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join(&sha256), b"corrupt").unwrap();
    let unrelated = bad.join(Sha256::of(b"unrelated").to_string());
    fs::write(&unrelated, b"unrelated").unwrap();
    fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o000)).unwrap();

    let (code, doc) = run(&dir, &netlist, &[&bad]);
    assert_eq!(
        (code, doc["status"].as_str()),
        (0, Some("ok")),
        "a good copy beside the netlist, a bad one under --blobs: {doc}"
    );

    let beside = dir.join("blobs").join(&sha256);
    fs::set_permissions(&beside, fs::Permissions::from_mode(0o000)).unwrap();
    let (code, doc) = run(&dir, &netlist, &[&bad, &example_blobs()]);
    assert_eq!(
        (code, doc["status"].as_str()),
        (0, Some("ok")),
        "an unreadable copy beside the netlist, a good one under the second --blobs: {doc}"
    );
    let (code, doc) = run(&dir, &netlist, &[&bad]);
    assert_eq!(
        (code, doc["status"].as_str()),
        (3, Some("io_error")),
        "an unreadable copy and a bad one: {doc}"
    );

    fs::remove_dir_all(dir.join("blobs")).unwrap();
    let (code, doc) = run(&dir, &netlist, &[&bad, &example_blobs()]);
    assert_eq!(
        (code, doc["status"].as_str()),
        (0, Some("ok")),
        "a bad copy under the first --blobs, a good one under the second: {doc}"
    );
}

#[test]
fn a_blob_that_is_not_an_executable_for_its_ram_is_a_firmware_error() {
    let dir = scratch("not-elf");
    let mut netlist = example();
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    for (name, bytes) in [
        ("elf", b"not an executable".to_vec()),
        ("unmapped", elf(0x4000_0000, 0x4000_0000, &[0; 4], &[])),
    ] {
        let sha256 = Sha256::of(&bytes).to_string();
        fs::write(blobs.join(&sha256), &bytes).unwrap();
        firmware(&mut netlist, &sha256, bytes.len());
        let (code, doc) = run(&dir, &netlist, &[]);
        assert_eq!(code, 1, "{name}");
        let found = diagnostics(&doc);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert_eq!(found[0]["code"], "netlist.firmware", "{name}");
        assert_eq!(found[0]["data"]["rule"], name);
    }
}

#[test]
fn a_gpio_is_optional_and_a_pin_the_chip_lacks_is_refused() {
    let dir = scratch("pins");
    let mut netlist = example();
    netlist["parts"][1]["pins"] = serde_json::json!({ "VCC": "vcc", "GND": "gnd" });
    let (code, doc) = run(&dir, &netlist, &[&example_blobs()]);
    assert_eq!(code, 0, "no GPIO has a net: {doc}");
    netlist["parts"][1]["pins"] = serde_json::json!({ "VCC": "vcc", "GPIO22": "gpio21" });
    let (code, doc) = run(&dir, &netlist, &[&example_blobs()]);
    assert_eq!(code, 1);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "netlist.pins");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "missing": ["GND"], "unexpected": ["GPIO22"] })
    );
}

/// The part `docs/netlist.md` shows, with every value's source dropped.
fn without_sources(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .filter(|(key, _)| key.as_str() != "source")
            .map(|(key, value)| (key.clone(), without_sources(value)))
            .collect(),
        other => other.clone(),
    }
}

#[test]
fn the_documented_part_is_the_examples() {
    let doc = fs::read_to_string(repo("docs/netlist.md")).unwrap();
    let section = &doc[doc.find("### ESP32-S3").unwrap()..];
    let start = section.find("```json\n").unwrap() + "```json\n".len();
    let end = start + section[start..].find("\n```").unwrap();
    let documented: serde_json::Value = serde_json::from_str(&section[start..end]).unwrap();
    assert_eq!(
        without_sources(&documented),
        without_sources(&example()["parts"][1])
    );
}

const IRAM: u32 = 0x4038_0000;
/// `waiti 0`, which no interrupt ends, then `j` to itself.
const IDLE: [u8; 6] = [0x00, 0x70, 0x00, 0x06, 0xff, 0xff];

/// An ELF that runs `code` from `IRAM`, with `literals` in the words before it.
fn program(literals: &[u32], code: &[u8]) -> Vec<u8> {
    let mut image: Vec<u8> = literals
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let entry = IRAM + image.len() as u32;
    image.extend(code);
    elf(IRAM, entry, &image, &[("literals", IRAM), ("main", entry)])
}

/// The example's netlist with `bytes` as its firmware, stopping at `stop_ps`.
fn netlist_for(bytes: &[u8], stop_ps: u64) -> serde_json::Value {
    let mut netlist = example();
    firmware(&mut netlist, &Sha256::of(bytes).to_string(), bytes.len());
    netlist["analysis"]["transient"]["stop_ps"] = stop_ps.into();
    netlist
}

/// Runs `code` from `IRAM`, with `literals` in the words before it, on the example's board for
/// `stop_ps`.
fn run_program(name: &str, literals: &[u32], code: &[u8], stop_ps: u64) -> serde_json::Value {
    let dir = scratch(name);
    let bytes = program(literals, code);
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (_, doc) = run(&dir, &netlist_for(&bytes, stop_ps), &[]);
    doc
}

/// The `ws2812-replay` frame, moved so its reset latches before timer group 0 runs out, on the
/// example board running `IDLE`.
fn replay_on(netlist: &mut serde_json::Value, event_ps: u64) {
    let replay: serde_json::Value =
        serde_json::from_slice(&fs::read(repo("examples/ws2812-replay/netlist.json")).unwrap())
            .unwrap();
    let nets = netlist["nets"].as_array_mut().unwrap();
    for name in ["din", "v5"] {
        nets.push(name.into());
    }
    let parts = netlist["parts"].as_array_mut().unwrap();
    for part in replay["parts"].as_array().unwrap() {
        match part["kind"].as_str() {
            Some("voltage_source") => {
                let mut source = part.clone();
                // The blink netlist already names its source `v1`.
                source["name"] = "v2".into();
                parts.push(source);
            }
            Some("pulses_replay") => {
                let mut sender = part.clone();
                let event = &mut sender["events"][0];
                assert_eq!(event["t_ps"], 1_000_000_000, "the replay example's event");
                event["t_ps"] = event_ps.into();
                parts.push(sender);
            }
            Some("ws2812") => parts.push(part.clone()),
            other => panic!("the replay example gained a part {other:?}"),
        }
    }
}

fn run_idle(name: &str, stop_ps: u64, event_ps: u64) -> (i32, serde_json::Value, String) {
    let bytes = program(&[], &IDLE);
    let mut netlist = netlist_for(&bytes, stop_ps);
    replay_on(&mut netlist, event_ps);
    let dir = scratch(name);
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist, &[]);
    let leds = fs::read_to_string(dir.join("out/traces/leds.csv")).unwrap();
    (code, doc, leds)
}

#[test]
fn a_watchdog_left_running_ends_the_run_at_its_first_stage() {
    let doc = run_program("watchdog", &[], &IDLE, 900_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.watchdog");
    assert_eq!(found[0]["severity"], "error");
    // Timer group 0 at its boot hold of 26,000,000 cycles of the 40 MHz clock, 0.65 s.
    assert_eq!(found[0]["t_ps"], 650_000_000_000u64);
    assert_eq!(found[0]["data"]["watchdog"], "timer group 0");
}

/// The replay frame's reset latches while its record is still open, and timer group 0 runs out
/// before that record ends. The faulted run's `leds.csv` is the file a stop just before the
/// fault writes.
#[test]
fn a_watchdog_during_a_pulses_record_keeps_the_leds_a_stop_keeps() {
    // The example latches at 1_669_062_501. Moving its event from 1 ms to 649.32 ms moves the
    // latch to 649_989_062_501, before the 650 ms fault and before the record ends at 650.01 ms.
    const EVENT_PS: u64 = 649_320_000_000;
    const LATCH: &str = "649989062501,ring,13,0010200111210212220313230414240515250616260717270818280919290a1a2a0b1b2b0c1c2c";
    let (code, doc, stopped) = run_idle("latch-stop", 649_999_000_000, EVENT_PS);
    assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
    let (code, doc, faulted) = run_idle("latch-fault", 900_000_000_000, EVENT_PS);
    assert_eq!((code, doc["status"].as_str()), (1, Some("failed")), "{doc}");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.watchdog");
    assert_eq!(found[0]["t_ps"], 650_000_000_000u64);
    assert_eq!(faulted, stopped);
    let mut lines = faulted.lines();
    assert_eq!(lines.next(), Some("t_ps,part,count,grb"));
    assert_eq!(lines.next(), Some(LATCH));
    assert!(lines.next().is_none(), "{faulted}");
}

#[test]
fn an_unmodeled_register_is_reported_once_with_its_pc_and_symbol() {
    // l32r a2 <- UART0; l32i a3, a2, 0 twice; then idle.
    let mut code = vec![0x21, 0xff, 0xff, 0x32, 0x22, 0x00, 0x32, 0x22, 0x00];
    code.extend(IDLE);
    let doc = run_program("unmodeled", &[0x6000_0000], &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unmodeled_register");
    assert_eq!(found[0]["severity"], "warning");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "address": 0x6000_0000u32,
            "pc": IRAM + 4 + 3,
            "register": "UART0+0x0",
            "symbol": "main+0x3",
            "write": false,
        })
    );
    // The l32r takes 1 cycle at 25 ns, so the read is at cycle 1.
    assert_eq!(found[0]["t_ps"], 25_000);
}

#[test]
fn a_session_lists_an_unmodeled_register_only_once_its_access_is_accepted() {
    // l32r a2 <- 100000; addi a2, a2, -1 and bnez back to it until a2 is 0; l32r a3 <- UART0;
    // l32i a4, a3, 0; then idle.
    let mut code = vec![
        0x21, 0xfe, 0xff, 0x22, 0xc2, 0xff, 0x56, 0x92, 0xff, 0x31, 0xfc, 0xff, 0x42, 0x23, 0x00,
    ];
    code.extend(IDLE);
    let bytes = program(&[100_000, 0x6000_0000], &code);
    let json = serde_json::to_vec(&netlist_for(&bytes, 100_000_000_000)).unwrap();
    let mut blobs = Blobs::new();
    blobs.add(bytes.into());
    let touched = |session: &Session| -> Vec<Time> {
        session
            .diagnostics()
            .iter()
            .filter(|found| found.code() == "chip.unmodeled_register")
            .map(|found| found.at().expect("at a time"))
            .collect()
    };

    let mut session = Session::open_document_with(&json, &blobs).unwrap();
    session.advance_to(Time::from_ps(u64::MAX));
    let [at] = touched(&session)[..] else {
        panic!("one register touched: {:?}", session.diagnostics());
    };
    assert!(
        at > Time::from_ps(1_000_000_000),
        "after the loop, at {at:?}"
    );

    let mut session = Session::open_document_with(&json, &blobs).unwrap();
    session.advance_to(Time::from_ps(at.as_ps() - 1));
    assert_eq!(
        touched(&session),
        [],
        "the copy ahead touched it, the run has not"
    );
    session.advance_to(Time::from_ps(u64::MAX));
    assert_eq!(touched(&session), [at]);
}

#[test]
fn an_op_the_core_does_not_run_ends_the_run() {
    // movi a2, 0; rfi 1, which only levels 2 to 7 return from.
    let doc = run_program(
        "unsupported",
        &[0],
        &[0x22, 0xa0, 0x00, 0x10, 0x31, 0x00],
        10_000_000_000,
    );
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_instruction");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["refs"], serde_json::json!(["#u1"]));
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "op": "rfi", "pc": IRAM + 4 + 3, "reason": "op", "symbol": "main+0x3" })
    );
    // The movi takes 1 cycle at 25 ns.
    assert_eq!(found[0]["t_ps"], 25_000);
}

/// An FPU op runs once `CPENABLE` bit 0 is set, so the run goes on to the `rfi` after it. With
/// the bit clear the op raises cause 32. `PS.EXCM` is still set from reset, so the core takes it
/// at the double exception vector, `VECBASE + 0x3c0`, where nothing is mapped.
#[test]
fn an_fpu_op_runs_once_its_coprocessor_is_enabled() {
    // movi.n a2, 1; wsr.cpenable a2; rsync; add.s f0, f1, f2; rfi 1.
    let doc = run_program(
        "fpu",
        &[0],
        &[
            0x0c, 0x12, 0x20, 0xe0, 0x13, 0x10, 0x20, 0x00, 0x20, 0x01, 0x0a, 0x10, 0x31, 0x00,
        ],
        10_000_000_000,
    );
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_instruction");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "op": "rfi", "pc": IRAM + 4 + 11, "reason": "op", "symbol": "main+0xb" })
    );

    // movi.n a2, 0; wsr.cpenable a2; rsync; add.s f0, f1, f2; rfi 1.
    let doc = run_program(
        "fpu-off",
        &[0],
        &[
            0x0c, 0x02, 0x20, 0xe0, 0x13, 0x10, 0x20, 0x00, 0x20, 0x01, 0x0a, 0x10, 0x31, 0x00,
        ],
        10_000_000_000,
    );
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unmapped_fetch");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "pc": 0x4000_03c0, "symbol": null })
    );
}

#[test]
fn a_jump_out_of_the_instruction_window_ends_the_run() {
    // movi a2, 0; jx a2.
    let doc = run_program(
        "unmapped",
        &[],
        &[0x22, 0xa0, 0x00, 0xa0, 0x02, 0x00],
        10_000_000_000,
    );
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unmapped_fetch");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "pc": 0, "symbol": null })
    );
}

/// A 32-bit little-endian Xtensa executable: one loadable segment of `image` at `addr`, entry
/// `entry`, and one function symbol per `(name, addr)`.
fn elf(addr: u32, entry: u32, image: &[u8], symbols: &[(&str, u32)]) -> Vec<u8> {
    let (phoff, data_at) = (52u32, 52 + 32);
    let mut file = vec![0u8; 52];
    file[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    file[16..18].copy_from_slice(&2u16.to_le_bytes());
    file[18..20].copy_from_slice(&94u16.to_le_bytes());
    file[24..28].copy_from_slice(&entry.to_le_bytes());
    file[28..32].copy_from_slice(&phoff.to_le_bytes());
    file[42..44].copy_from_slice(&32u16.to_le_bytes());
    file[44..46].copy_from_slice(&1u16.to_le_bytes());
    let size = image.len() as u32;
    for word in [1, data_at, addr, addr, size, size, 5, 4] {
        file.extend(word.to_le_bytes());
    }
    file.extend(image);
    let mut strings = vec![0u8];
    let mut table = vec![0u8; 16];
    for &(name, at) in symbols {
        table.extend((strings.len() as u32).to_le_bytes());
        strings.extend(name.as_bytes());
        strings.push(0);
        table.extend(at.to_le_bytes());
        table.extend(0u32.to_le_bytes());
        table.extend([0x12, 0]);
        table.extend(1u16.to_le_bytes());
    }
    let table_at = file.len() as u32;
    file.extend(&table);
    let strings_at = file.len() as u32;
    file.extend(&strings);
    let shoff = file.len() as u32;
    file[32..36].copy_from_slice(&shoff.to_le_bytes());
    file[46..48].copy_from_slice(&40u16.to_le_bytes());
    file[48..50].copy_from_slice(&3u16.to_le_bytes());
    for (kind, offset, size, link) in [
        (0u32, 0, 0, 0u32),
        (2, table_at, table.len() as u32, 2),
        (3, strings_at, strings.len() as u32, 0),
    ] {
        let mut header = [0u8; 40];
        header[4..8].copy_from_slice(&kind.to_le_bytes());
        header[16..20].copy_from_slice(&offset.to_le_bytes());
        header[20..24].copy_from_slice(&size.to_le_bytes());
        header[24..28].copy_from_slice(&link.to_le_bytes());
        file.extend(header);
    }
    file
}
