//! `esp32s3` parts through `orbe-run run`: where the firmware blob is found, the `netlist.blob` error
//! when it is not, and the diagnostics a program raises when it leaves a watchdog running,
//! touches a register the model does not keep, reaches an instruction the core does not run,
//! starts an SPI2 transfer the model does not run, or jumps out of its memory, and an FPU op the
//! core now runs.

use std::fs;
use std::path::{Path, PathBuf};

use orbe_model::Time;
use orbe_scratch::Scratch;

use crate::runs::common;
use common::{repo, run_netlist};
use orbe_engine::blob::{Blobs, Sha256};
use orbe_engine::{Refused, Session};

fn example() -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(repo(
            "packages/engine/tests/circuits/esp32s3-blink/netlist.json",
        ))
        .unwrap(),
    )
    .unwrap()
}

fn example_blobs() -> PathBuf {
    repo("packages/engine/tests/circuits/esp32s3-blink/blobs")
}

/// Writes `netlist` into `dir` and runs it, with `--blobs` for each of `blobs`.
fn run(dir: &Path, netlist: &serde_json::Value, blobs: &[&Path]) -> (u8, serde_json::Value) {
    let path = dir.join("netlist.json");
    fs::write(&path, serde_json::to_vec_pretty(netlist).unwrap()).unwrap();
    let blobs: Vec<PathBuf> = blobs.iter().map(|blob| blob.to_path_buf()).collect();
    run_netlist(&path, &dir.join("out"), &blobs)
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
    let dir = Scratch::new();
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
    let dir = Scratch::new();
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
        let Err(Refused::Invalid(found)) = Session::open_json(&json, blobs) else {
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

    let dir = Scratch::new();
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
    let dir = Scratch::new();
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

fn hello() -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(repo(
            "packages/engine/tests/circuits/esp32s3-hello/netlist.json",
        ))
        .unwrap(),
    )
    .unwrap()
}

/// `rom` goes with `firmware.flash` and only there, and is the pinned ROM, which no example
/// commits, so its absence is the one blob a run of the flash example names.
#[test]
fn the_rom_comes_with_a_flash_and_is_the_pinned_one() {
    let dir = Scratch::new();
    let hello_blobs = repo("packages/engine/tests/circuits/esp32s3-hello/blobs");
    let rom = hello()["parts"][1]["rom"].clone();
    let mut other = hello();
    other["parts"][1]["rom"]["bytes"] = 949_553.into();
    let mut absent = hello();
    absent["parts"][1].as_object_mut().unwrap().remove("rom");
    let mut beside_elf = example();
    beside_elf["parts"][1]["rom"] = rom.clone();
    for (name, netlist) in [
        ("other", other),
        ("absent", absent),
        ("beside an elf", beside_elf),
    ] {
        let (code, doc) = run(&dir, &netlist, &[&hello_blobs, &example_blobs()]);
        assert_eq!(code, 1, "{name}");
        let found = diagnostics(&doc);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert_eq!(found[0]["code"], "netlist.firmware", "{name}");
        assert_eq!(found[0]["data"]["rule"], "rom", "{name}");
    }
    let (code, doc) = run(&dir, &hello(), &[&hello_blobs]);
    assert_eq!(code, 1);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "netlist.blob");
    assert_eq!(found[0]["data"]["role"], "rom");
    assert_eq!(found[0]["data"]["sha256"], rom["sha256"]);
}

#[test]
fn a_gpio_is_optional_and_a_pin_the_chip_lacks_is_refused() {
    let dir = Scratch::new();
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

#[test]
fn the_documented_part_is_the_examples() {
    let doc = fs::read_to_string(repo("docs/engine/netlist.md")).unwrap();
    let section = &doc[doc.find("### ESP32-S3").unwrap()..];
    let start = section.find("```json\n").unwrap() + "```json\n".len();
    let end = start + section[start..].find("\n```").unwrap();
    let documented: serde_json::Value = serde_json::from_str(&section[start..end]).unwrap();
    assert_eq!(documented, example()["parts"][1]);
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
fn run_program(literals: &[u32], code: &[u8], stop_ps: u64) -> serde_json::Value {
    let dir = Scratch::new();
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
    let replay: serde_json::Value = serde_json::from_slice(
        &fs::read(repo(
            "packages/engine/tests/circuits/ws2812-replay/netlist.json",
        ))
        .unwrap(),
    )
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

fn run_idle(stop_ps: u64, event_ps: u64) -> (u8, serde_json::Value, String) {
    let bytes = program(&[], &IDLE);
    let mut netlist = netlist_for(&bytes, stop_ps);
    replay_on(&mut netlist, event_ps);
    let dir = Scratch::new();
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist, &[]);
    let leds = fs::read_to_string(dir.join("out/traces/leds.csv")).unwrap();
    (code, doc, leds)
}

#[test]
fn a_watchdog_left_running_ends_the_run_at_its_first_stage() {
    let doc = run_program(&[], &IDLE, 900_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.watchdog");
    assert_eq!(found[0]["severity"], "error");
    // Timer group 0 at its boot hold of 26,000,000 cycles of the 40 MHz clock, 0.65 s, in flash
    // boot mode, which resets the system.
    assert_eq!(found[0]["t_ps"], 650_000_000_000u64);
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "reset": "system", "stage": 0, "watchdog": "timer group 0" })
    );
    assert_eq!(
        found[0]["message"],
        "#u1 timer group 0 watchdog's stage 0 ran out and reset the system at 650000000000 ps; the firmware neither fed nor disabled it"
    );
}

/// Code that stores each value to its address in turn, store `k` at 40 MHz cycle `3k + 2`, then
/// idles, and its literals.
fn stores(pairs: &[(u32, u32)]) -> (Vec<u32>, Vec<u8>) {
    let literals: Vec<u32> = pairs.iter().flat_map(|&(a, v)| [a, v]).collect();
    let entry = IRAM + 4 * literals.len() as u32;
    let mut code = Vec::new();
    for k in 0..pairs.len() as u32 {
        for t in 0..2 {
            let pc = entry + code.len() as u32;
            let literal = IRAM + 4 * (2 * k + t);
            let imm = (literal.wrapping_sub((pc + 3) & !3) as i32 >> 2) as u16;
            code.extend([(2 + t as u8) << 4 | 1, imm as u8, (imm >> 8) as u8]);
        }
        // s32i a3, a2, 0
        code.extend([0x32, 0x62, 0x00]);
    }
    code.extend(IDLE);
    (literals, code)
}

/// Timer group 0's stage 0 interrupts 1000 cycles after the store that starts it, store 4 at
/// cycle 14, and nothing takes the interrupt. Stage 1 resets the system 500 cycles later, at
/// cycle 1514.
#[test]
fn a_watchdog_stage_that_interrupts_leads_to_the_stage_that_resets() {
    let (literals, code) = stores(&[
        (0x6000_8098, 0),
        (0x6001_f048, 0),
        (0x6001_f050, 1000),
        (0x6001_f054, 500),
        (0x6001_f048, 1 << 31 | 1 << 29 | 3 << 27),
    ]);
    let doc = run_program(&literals, &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.watchdog");
    assert_eq!(found[0]["t_ps"], 1514 * 25_000);
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "reset": "system", "stage": 1, "watchdog": "timer group 0" })
    );
}

/// The `u1` `GPIO21` changes in `logic.vcd`, as each time and level.
fn gpio21_edges(vcd: &str) -> Vec<(u64, char)> {
    let mut scope = "";
    let mut id = None;
    for line in vcd.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["$scope", "module", name, "$end"] => scope = name,
            ["$var", "wire", "1", code, "GPIO21", "$end"] if scope == "u1" => id = Some(code),
            _ => {}
        }
    }
    let id = id.expect("GPIO21 in logic.vcd");
    let mut at = 0;
    let mut edges = Vec::new();
    for line in vcd.lines().skip_while(|line| *line != "$end") {
        if let Some(time) = line.strip_prefix('#') {
            at = time.parse().unwrap();
        } else if line.len() == 1 + id.len() && line.ends_with(id) && at > 0 {
            edges.push((at, line.chars().next().unwrap()));
        }
    }
    edges
}

/// LEDC channel 0 on GPIO21, timer 0 on the 40 MHz crystal at 1 bit and divider 1, a period of
/// two cycles, 50 ns, from store 5 at cycle 17. The duty goes from 0 to 1 of 2 steps at store
/// 8, cycle 26, and back to 0 at store 11, cycle 35. Each takes effect at the timer's next
/// overflow, at cycles 27 and 37, 675 and 925 ns: a `Duty` row then. Between them GPIO21 is
/// high for the first 25 ns of each period, each edge 1 ps after its stop. The update at 925 ns
/// finds the pin low and moves nothing.
#[test]
fn an_led_duty_from_0_to_half_and_back_sends_two_duty_rows_and_its_pwm() {
    let ledc = 0x6001_9000;
    let duty = |duty: u32| {
        [
            (ledc + 0x08, duty << 4),
            (ledc + 0x0c, 1 << 31 | 1 << 30 | 1 << 20 | 1 << 10),
            (ledc, 1 << 2 | 1 << 4),
        ]
    };
    let setup = [
        (0x6000_9058, 1 << 12 | 3 << 10 | 1 << 9),
        (0x6000_45a8, 73),
        (0x6000_4024, 1 << 21),
        (0x600c_0018, 0xf9c1_e06f | 1 << 11),
        (ledc + 0xd0, 3),
        (ledc + 0xa0, 1 | 256 << 4 | 1 << 25),
    ];
    let (literals, code) = stores(&[&setup[..], &duty(1), &duty(0)].concat());
    let dir = Scratch::new();
    let bytes = program(&literals, &code);
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist_for(&bytes, 10_000_000_000), &[]);
    assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
    let link = fs::read_to_string(dir.join("out/traces/link.csv")).unwrap();
    assert_eq!(
        link,
        "t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash,lanes\n\
         675000,675000,u1,GPIO21,duty,,1/2@20000000,,,\n\
         925000,925000,u1,GPIO21,duty,,0/2@20000000,,,\n"
    );
    let vcd = fs::read_to_string(dir.join("out/traces/logic.vcd")).unwrap();
    let pwm: Vec<(u64, char)> = (0..10)
        .map(|k| (675_001 + 25_000 * k, if k % 2 == 0 { '1' } else { '0' }))
        .collect();
    assert_eq!(gpio21_edges(&vcd), pwm);
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
    let (code, doc, stopped) = run_idle(649_999_000_000, EVENT_PS);
    assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
    let (code, doc, faulted) = run_idle(900_000_000_000, EVENT_PS);
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
    // l32r a2 <- RTC_IO_RTC_GPIO_OUT_REG; l32i a3, a2, 0 twice; then idle.
    let mut code = vec![0x21, 0xff, 0xff, 0x32, 0x22, 0x00, 0x32, 0x22, 0x00];
    code.extend(IDLE);
    let doc = run_program(&[0x6000_8400], &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unmodeled_register");
    assert_eq!(found[0]["severity"], "warning");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "address": 0x6000_8400u32,
            "kept": false,
            "pc": IRAM + 4 + 3,
            "register": "RTCIO+0x0",
            "symbol": "main+0x3",
            "write": false,
        })
    );
    // The l32r takes 1 cycle at 25 ns, so the read is at cycle 1.
    assert_eq!(found[0]["t_ps"], 25_000);
}

#[test]
fn a_session_lists_an_unmodeled_register_only_once_its_access_is_accepted() {
    // l32r a2 <- 100000; addi a2, a2, -1 and bnez back to it until a2 is 0;
    // l32r a3 <- RTC_IO_RTC_GPIO_OUT_REG; l32i a4, a3, 0; then idle.
    let mut code = vec![
        0x21, 0xfe, 0xff, 0x22, 0xc2, 0xff, 0x56, 0x92, 0xff, 0x31, 0xfc, 0xff, 0x42, 0x23, 0x00,
    ];
    code.extend(IDLE);
    let bytes = program(&[100_000, 0x6000_8400], &code);
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

    let mut session = Session::open_json(&json, &blobs).unwrap();
    session.advance_to(Time::from_ps(u64::MAX));
    let [at] = touched(&session)[..] else {
        panic!("one register touched: {:?}", session.diagnostics());
    };
    assert!(
        at > Time::from_ps(1_000_000_000),
        "after the loop, at {at:?}"
    );

    let mut session = Session::open_json(&json, &blobs).unwrap();
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
    let doc = run_program(&[0], &[0x22, 0xa0, 0x00, 0x10, 0x31, 0x00], 10_000_000_000);
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

/// A store of `EFUSE_PGM_CMD` to `EFUSE_CMD_REG` would burn eFuses, so it ends the run.
#[test]
fn an_efuse_program_command_ends_the_run() {
    // l32r a2 <- EFUSE_CONF_REG; l32r a3 <- its program opcode; s32i a3, a2, 0;
    // l32r a2 <- EFUSE_CMD_REG; l32r a3 <- EFUSE_PGM_CMD; s32i a3, a2, 0; then idle.
    let mut code = vec![
        0x21, 0xfc, 0xff, 0x31, 0xfc, 0xff, 0x32, 0x62, 0x00, 0x21, 0xfb, 0xff, 0x31, 0xfc, 0xff,
        0x32, 0x62, 0x00,
    ];
    code.extend(IDLE);
    let literals = [0x6000_71cc, 0x5a5a, 0x6000_71d4, 0b10];
    let doc = run_program(&literals, &code, 10_000_000_000);
    assert_eq!(doc["status"], "failed");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.efuse_program");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "pc": IRAM + 16 + 15, "symbol": "main+0xf" })
    );
    // Five ops of one cycle at 25 ns come before the store.
    assert_eq!(found[0]["t_ps"], 125_000);
}

/// A bare-metal chip has no flash, so a sector erase through SPI1 reaches past the flash's end
/// and ends the run with its command byte.
#[test]
fn a_flash_erase_with_no_flash_ends_the_run() {
    // l32r a2 <- SPI_MEM_CMD_REG of SPI1; l32r a3 <- SPI_MEM_FLASH_SE; s32i a3, a2, 0; then idle.
    let mut code = vec![0x21, 0xfe, 0xff, 0x31, 0xfe, 0xff, 0x32, 0x62, 0x00];
    code.extend(IDLE);
    let doc = run_program(&[0x6000_2000, 1 << 24], &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.flash_command");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "command": 0x20,
            "pc": IRAM + 8 + 6,
            "reason": "past_end",
            "symbol": "main+0x6"
        })
    );
    let message = found[0]["message"].as_str().unwrap();
    assert!(
        message.contains("sent the flash command 0x20 at ")
            && message.contains("which reaches past the flash's end"),
        "{message}"
    );
}

/// A pad of the flash given to the GPIO matrix could send the flash any command, so that write
/// ends the run.
#[test]
fn a_write_that_takes_a_flash_pad_ends_the_run() {
    // l32r a2 <- IO_MUX_GPIO30_REG; l32r a3 <- function 1, the GPIO; s32i a3, a2, 0; then idle.
    let mut code = vec![0x21, 0xfe, 0xff, 0x31, 0xfe, 0xff, 0x32, 0x62, 0x00];
    code.extend(IDLE);
    let doc = run_program(&[0x6000_907c, 1 << 12], &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.flash_command");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "command": null,
            "pc": IRAM + 8 + 6,
            "reason": "pads",
            "symbol": "main+0x6"
        })
    );
    let message = found[0]["message"].as_str().unwrap();
    assert!(
        message.contains("took a flash pad from SPI0 and SPI1 at 0x4038000e in main+0x6, which could send the flash any command"),
        "{message}"
    );
}

/// `RTC_CNTL_SW_SYS_RST` resets the chip. A bare-metal chip keeps no `RTC_CNTL_STORE6_REG`, so
/// the model cannot read why, and the reset ends the run as an error. The hello example covers a
/// reset from `esp_restart`, which holds the chip in reset to the run's stop.
#[test]
fn a_reset_with_no_reason_the_model_can_read_fails_the_run() {
    // l32r a2 <- RTC_CNTL_OPTIONS0_REG; l32r a3 <- RTC_CNTL_SW_SYS_RST; s32i a3, a2, 0; then idle.
    let mut code = vec![0x21, 0xfe, 0xff, 0x31, 0xfe, 0xff, 0x32, 0x62, 0x00];
    code.extend(IDLE);
    let doc = run_program(&[0x6000_8000, 1 << 31], &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.fault_restart");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "pc": IRAM + 8 + 6, "reason": "unreadable", "symbol": "main+0x6" })
    );
    assert_eq!(found[0]["t_ps"], 50_000);
    assert_eq!(doc["status"], "failed");
}

/// The CPU leaves the 40 MHz crystal for the 240 MHz PLL at the store that selects it, so the read
/// two cycles later comes two 240 MHz cycles after it.
#[test]
fn a_switch_to_the_pll_speeds_up_the_core() {
    // l32r a2 <- SYSTEM_CPU_PER_CONF_REG; l32r a3 <- 240 MHz from the 480 MHz PLL; s32i a3, a2, 0;
    // l32r a2 <- SYSTEM_SYSCLK_CONF_REG; l32r a3 <- the PLL; s32i a3, a2, 0;
    // l32r a2 <- RTC_IO_RTC_GPIO_OUT_REG; l32i a3, a2, 0; then idle.
    let mut code = vec![
        0x21, 0xfb, 0xff, 0x31, 0xfb, 0xff, 0x32, 0x62, 0x00, 0x21, 0xfa, 0xff, 0x31, 0xfb, 0xff,
        0x32, 0x62, 0x00, 0x21, 0xfa, 0xff, 0x32, 0x22, 0x00,
    ];
    code.extend(IDLE);
    let literals = [0x600c_0010, 0x6, 0x600c_0060, 1 << 10, 0x6000_8400];
    let doc = run_program(&literals, &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unmodeled_register");
    assert_eq!(found[0]["data"]["pc"], IRAM + 20 + 21);
    // The store at cycle 5 is 60 ticks of 480 MHz, and two 240 MHz cycles of 2 ticks follow: 64
    // ticks, 133333.3 ps, rounded up.
    assert_eq!(found[0]["t_ps"], 133_334);
}

/// The RC fast clock is not one the model runs the CPU on.
#[test]
fn a_switch_to_the_rc_fast_clock_ends_the_run() {
    // l32r a2 <- SYSTEM_SYSCLK_CONF_REG; l32r a3 <- the RC fast clock; s32i a3, a2, 0; then idle.
    let mut code = vec![0x21, 0xfe, 0xff, 0x31, 0xfe, 0xff, 0x32, 0x62, 0x00];
    code.extend(IDLE);
    let doc = run_program(&[0x600c_0060, 2 << 10], &code, 10_000_000_000);
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_clock");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({ "pc": IRAM + 8 + 6, "symbol": "main+0x6" })
    );
}

/// An FPU op runs once `CPENABLE` bit 0 is set, so the run goes on to the `rfi` after it. With
/// the bit clear the op raises cause 32. `PS.EXCM` is still set from reset, so the core takes it
/// at the double exception vector, `VECBASE + 0x3c0`, where nothing is mapped.
#[test]
fn an_fpu_op_runs_once_its_coprocessor_is_enabled() {
    // movi.n a2, 1; wsr.cpenable a2; rsync; add.s f0, f1, f2; rfi 1.
    let doc = run_program(
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
fn an_spi2_transfer_with_a_setting_the_model_does_not_run_ends_the_run() {
    // l32r a2 <- SPI2; l32r a3 <- USR_DUMMY; l32r a4 <- USR; s32i a3 to USER; s32i a4 to CMD.
    let code = [
        0x21, 0xfd, 0xff, 0x31, 0xfd, 0xff, 0x41, 0xfd, 0xff, 0x32, 0x62, 0x04, 0x42, 0x62, 0x00,
    ];
    let doc = run_program(
        &[0x6002_4000, 1 << 29, 1 << 24],
        &[code.as_slice(), &IDLE].concat(),
        10_000_000_000,
    );
    assert_eq!(doc["status"], "failed", "{doc}");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_setting");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["refs"], serde_json::json!(["#u1"]));
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "peripheral": "SPI2",
            "pc": IRAM + 12 + 12,
            "setting": "dummy",
            "symbol": "main+0xc",
        })
    );
}

/// The blink part names no `pull_down_resistance`, and GPIO21 has a net. A store of `FUN_WPD`
/// to its IO_MUX word ends the run and writes nothing.
#[test]
fn a_pull_down_on_a_part_that_names_none_ends_the_run() {
    // l32r a2 <- GPIO21's IO_MUX; l32r a3 <- FUN_WPD; s32i a3, a2, 0.
    let doc = run_program(
        &[0x6000_9058, 1 << 7],
        &[
            [0x21, 0xfe, 0xff, 0x31, 0xfe, 0xff, 0x32, 0x62, 0x00].as_slice(),
            &IDLE,
        ]
        .concat(),
        10_000_000_000,
    );
    assert_eq!(doc["status"], "failed", "{doc}");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_setting");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["refs"], serde_json::json!(["#u1"]));
    assert_eq!(found[0]["t_ps"], 50_000);
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "peripheral": "IO_MUX",
            "pc": IRAM + 8 + 6,
            "setting": "pull_down",
            "symbol": "main+0x6",
        })
    );
    assert_eq!(
        found[0]["message"],
        "#u1 set IO_MUX pull_down at 0x4038000e in main+0x6, which the model does not run, at 50000 ps"
    );
}

/// Reading an unmodeled SENS touch register ends the run at that first access, naming the touch
/// sensor with no prior warning.
#[test]
fn a_touch_sensor_access_ends_the_run_naming_the_touch_sensor() {
    // l32r a2 <- SENS_SAR_TOUCH_CONF_REG; l32i a3, a2, 0; then idle.
    let mut read_code = vec![0x21, 0xff, 0xff, 0x32, 0x22, 0x00];
    read_code.extend(IDLE);
    let doc = run_program(&[0x6000_885c], &read_code, 10_000_000_000);
    assert_eq!(doc["status"], "failed", "{doc}");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_setting");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["data"]["peripheral"], "SENS");
    assert_eq!(found[0]["data"]["setting"], "unsimulated");
    let message = found[0]["message"].as_str().unwrap();
    assert!(
        message.contains("used SENS, the touch sensor that touchRead uses, at ")
            && message.contains("which the model does not simulate"),
        "{message}"
    );
}

/// ESP-IDF's light sleep suspends UART0 with `uart_ll_force_xoff` (`SW_FLOW_CON_EN | FORCE_XOFF`)
/// before writing `RTC_CNTL_SLEEP_EN`. The force-xoff completes so the run stops at the sleep
/// register and names sleep rather than `UART0 sw_flow`.
#[test]
fn light_sleep_uart_suspend_reaches_sleep_and_names_sleep() {
    let (literals, code) = stores(&[(0x6000_0034, 1 | 1 << 3), (0x6000_8018, 1 << 31)]);
    let doc = run_program(&literals, &code, 10_000_000_000);
    assert_eq!(doc["status"], "failed", "{doc}");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_setting");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(
        found[0]["data"],
        serde_json::json!({
            "peripheral": "RTC_CNTL",
            "pc": IRAM + 16 + 15,
            "setting": "sleep",
            "symbol": "main+0xf",
        })
    );
    let message = found[0]["message"].as_str().unwrap();
    assert!(
        message.contains("put the chip to sleep at ")
            && message.contains("which the model does not run"),
        "{message}"
    );
}

#[test]
fn a_jump_out_of_the_instruction_window_ends_the_run() {
    // movi a2, 0; jx a2.
    let doc = run_program(&[], &[0x22, 0xa0, 0x00, 0xa0, 0x02, 0x00], 10_000_000_000);
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

/// Polls `USB_SERIAL_JTAG_EP1_CONF_REG` until `SERIAL_OUT_EP_DATA_AVAIL`, takes the byte from
/// `USB_SERIAL_JTAG_EP1_REG`, waits for `SERIAL_IN_EP_DATA_FREE`, writes it back and `WR_DONE`,
/// and polls again.
const ECHO: [u8; 30] = [
    0x21, 0xff, 0xff, // l32r a2 <- USB_SERIAL_JTAG
    0x32, 0x22, 0x01, // l32i a3, a2, 4
    0x27, 0x63, 0xf9, // bbci a3, 2, back to the l32i
    0x42, 0x22, 0x00, // l32i a4, a2, 0
    0x32, 0x22, 0x01, // l32i a3, a2, 4
    0x17, 0x63, 0xf9, // bbci a3, 1, wait SERIAL_IN_EP_DATA_FREE
    0x42, 0x62, 0x00, // s32i a4, a2, 0
    0x52, 0xa0, 0x01, // movi a5, 1
    0x52, 0x62, 0x01, // s32i a5, a2, 4
    0x06, 0xf9, 0xff, // j back to the first l32i
];

#[test]
fn the_console_reads_each_byte_the_host_sends_from_its_time_on() {
    let bytes = program(&[0x6003_8000], &ECHO);
    let mut netlist = netlist_for(&bytes, 20_000_000_000);
    netlist["parts"][1]["console"] = serde_json::json!([
        { "t_ps": 1_000_000_000u64, "bytes": [65, 66] },
        { "t_ps": 1_500_000_001u64, "bytes": [67] },
    ]);
    let dir = Scratch::new();
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist, &[]);
    assert_eq!(code, 0, "{doc}");
    let serial = fs::read_to_string(dir.join("out/traces/serial.csv")).unwrap();
    let rows: Vec<(u64, u8)> = serial
        .lines()
        .skip(1)
        .map(|row| {
            let fields: Vec<&str> = row.split(',').collect();
            (fields[0].parse().unwrap(), fields[3].parse().unwrap())
        })
        .collect();
    let echoed: Vec<u8> = rows.iter().map(|&(_, byte)| byte).collect();
    assert_eq!(echoed, b"ABC", "{serial}");
    assert!(
        rows[0].0 > 1_000_000_000 && rows[1].0 > rows[0].0,
        "{serial}"
    );
    assert!(rows[2].0 > 1_500_000_001, "{serial}");
}

#[test]
fn a_console_send_at_t_0_out_of_order_or_empty_is_a_schedule_error() {
    let bytes = program(&[], &IDLE);
    let cases = [
        (
            serde_json::json!([{ "t_ps": 0, "bytes": [1] }]),
            0,
            "at_zero",
        ),
        (
            serde_json::json!([{ "t_ps": 5, "bytes": [1] }, { "t_ps": 5, "bytes": [2] }]),
            1,
            "not_increasing",
        ),
        (
            serde_json::json!([{ "t_ps": 5, "bytes": [] }]),
            0,
            "same_state",
        ),
    ];
    for (console, event, rule) in cases {
        let mut netlist = netlist_for(&bytes, 20_000_000_000);
        netlist["parts"][1]["console"] = console;
        let dir = Scratch::new();
        let blobs = dir.join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
        let (code, doc) = run(&dir, &netlist, &[]);
        assert_eq!(code, 1, "{doc}");
        let found = diagnostics(&doc);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["code"], "netlist.schedule");
        assert_eq!(found[0]["data"]["event"], event, "{rule}");
        assert_eq!(found[0]["data"]["rule"], rule);
    }
}

/// Replaces the blink LED load on `gpio21` with one pull-down resistor of `ohms` to `gnd`.
fn netlist_with_gpio21_resistor(bytes: &[u8], stop_ps: u64, ohms: f64) -> serde_json::Value {
    let mut netlist = netlist_for(bytes, stop_ps);
    netlist["nets"] = serde_json::json!(["gnd", "gpio21", "vcc"]);
    netlist["parts"].as_array_mut().unwrap().truncate(2);
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "r1",
            "kind": "resistor",
            "pins": { "a": "gpio21", "b": "gnd" },
            "resistance": { "value": ohms, "unit": "ohm" },
            "ratings": { "power": { "value": 0.25, "unit": "W" } }
        }));
    netlist
}

/// Writing `RUE` or `RDE` to an RTCIO pad register warns `chip.unmodeled_register` instead of
/// silently dropping the write, while clearing the pad register to 0 and setting `FUN_WPU` in
/// `IO_MUX` pulls the pin high with no diagnostic.
#[test]
fn an_rtcio_pad_pull_write_warns_while_io_mux_pull_drives_the_pin() {
    for bit in [1 << 27, 1 << 28] {
        let (literals, code) = stores(&[(0x6000_84d8, bit)]);
        let bytes = program(&literals, &code);
        let netlist = netlist_with_gpio21_resistor(&bytes, 10_000_000_000, 200_000.0);
        let dir = Scratch::new();
        let blobs = dir.join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
        let (code, doc) = run(&dir, &netlist, &[]);
        assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
        let found = diagnostics(&doc);
        assert_eq!(found.len(), 1, "bit {bit:#x}: {found:?}");
        assert_eq!(found[0]["code"], "chip.unmodeled_register");
        assert_eq!(found[0]["severity"], "warning");
        assert_eq!(found[0]["data"]["register"], "RTCIO+0xd8");
        assert_eq!(found[0]["data"]["write"], true);
        let vcd = fs::read_to_string(dir.join("out/traces/logic.vcd")).unwrap();
        assert_eq!(
            gpio21_edges(&vcd),
            [],
            "bit {bit:#x} does not pull digital pad"
        );
    }

    // Clearing the RTCIO pad register to 0 (as `rtc_gpio_deinit` does) and enabling `FUN_WPU`
    // in `IO_MUX_GPIO21_REG` pulls GPIO21 high with no diagnostic.
    let (literals, code) = stores(&[(0x6000_84d8, 0), (0x6000_9058, 1 << 12 | 1 << 9 | 1 << 8)]);
    let bytes = program(&literals, &code);
    let netlist = netlist_with_gpio21_resistor(&bytes, 10_000_000_000, 200_000.0);
    let dir = Scratch::new();
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist, &[]);
    assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
    assert!(diagnostics(&doc).is_empty(), "{doc}");
    let vcd = fs::read_to_string(dir.join("out/traces/logic.vcd")).unwrap();
    let edges = gpio21_edges(&vcd);
    assert_eq!(edges.len(), 1, "{vcd}");
    assert_eq!(edges[0].1, '1', "IO_MUX pull-up brings GPIO21 high");
}

/// Driving GPIO21 high, setting its hold bit in `RTC_CNTL_PAD_HOLD_REG`, and then attempting to
/// drive it low stops the run at the hold store with `chip.unsupported_setting` naming pin hold
/// (`gpio_hold_en`), before the later low write can move the pin.
#[test]
fn setting_a_pin_hold_bit_ends_the_run_before_a_later_write_moves_the_pin() {
    let (literals, code) = stores(&[
        (0x6000_9058, 1 << 12),
        (0x6000_4008, 1 << 21),
        (0x6000_4024, 1 << 21),
        (0x6000_80d8, 1 << 21),
        (0x6000_400c, 1 << 21),
    ]);
    let bytes = program(&literals, &code);
    let netlist = netlist_with_gpio21_resistor(&bytes, 10_000_000_000, 10_000.0);
    let dir = Scratch::new();
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist, &[]);
    assert_eq!((code, doc["status"].as_str()), (1, Some("failed")), "{doc}");
    let found = diagnostics(&doc);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "chip.unsupported_setting");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["data"]["peripheral"], "RTC_CNTL");
    assert_eq!(found[0]["data"]["setting"], "pin_hold");
    let message = found[0]["message"].as_str().unwrap();
    assert!(
        message.contains("pin hold (gpio_hold_en)"),
        "unexpected message: {message}"
    );
    let vcd = fs::read_to_string(dir.join("out/traces/logic.vcd")).unwrap();
    let edges = gpio21_edges(&vcd);
    assert_eq!(
        edges.len(),
        1,
        "GPIO21 rose high and never dropped low: {vcd}"
    );
    assert_eq!(edges[0].1, '1');

    // Deep-sleep autohold and force-hold bits in `RTC_CNTL_DIG_ISO_REG`, `RTC_CNTL_DIG_PAD_HOLD_REG`,
    // and `RTC_CNTL_PWC_REG` also stop the run with the same pin hold error.
    for (addr, value) in [
        (0x6000_8094, 1 << 11),
        (0x6000_8094, 1 << 15),
        (0x6000_80dc, 1 << 5),
        (0x6000_8088, 1 << 21),
    ] {
        let (literals, code) = stores(&[(addr, value)]);
        let doc = run_program(&literals, &code, 10_000_000_000);
        assert_eq!(doc["status"], "failed", "{addr:#x} = {value:#x}: {doc}");
        let found = diagnostics(&doc);
        assert_eq!(found.len(), 1, "{addr:#x} = {value:#x}: {found:?}");
        assert_eq!(found[0]["code"], "chip.unsupported_setting");
        assert_eq!(found[0]["data"]["peripheral"], "RTC_CNTL");
        assert_eq!(found[0]["data"]["setting"], "pin_hold");
        assert!(
            found[0]["message"]
                .as_str()
                .unwrap()
                .contains("pin hold (gpio_hold_en)"),
            "{addr:#x}: {}",
            found[0]["message"]
        );
    }
}

/// A 10,240-byte CDC burst (160 x 64-byte packets) polling `SERIAL_IN_EP_DATA_FREE` is paced by
/// the USB 2.0 Full-Speed host read rate (62.5 us per packet, 16 packets per 1 ms frame), so all
/// 160 packets arrive over >= 10 ms with every inter-packet gap >= 62.5 us.
#[test]
fn usb_cdc_burst_is_paced_at_full_speed_and_holds_data_free_low() {
    // l32r a2 <- USB_SERIAL_JTAG; l32r a3 <- 10240; movi a4, 'A';
    // loop: l32i a5, a2, 4; bbci a5, 1, loop; s32i a4, a2, 0; addi a3, a3, -1; bnez a3, loop;
    // j .
    let code: [u8; 27] = [
        0x21, 0xfe, 0xff, // l32r a2, lit0 (USB_SERIAL_JTAG)
        0x31, 0xfe, 0xff, // l32r a3, lit1 (10240)
        0x42, 0xa0, 0x41, // movi a4, 'A'
        0x52, 0x22, 0x01, // l32i a5, a2, 4 (EP1_CONF)
        0x17, 0x65, 0xf9, // bbci a5, 1, -7 (wait SERIAL_IN_EP_DATA_FREE)
        0x42, 0x62, 0x00, // s32i a4, a2, 0 (EP1)
        0x32, 0xc3, 0xff, // addi a3, a3, -1
        0x56, 0x03, 0xff, // bnez a3, -16
        0x06, 0xff, 0xff, // j .
    ];
    let bytes = program(&[0x6003_8000, 10_240], &code);
    let netlist = netlist_for(&bytes, 20_000_000_000);
    let dir = Scratch::new();
    let blobs = dir.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
    let (code, doc) = run(&dir, &netlist, &[]);
    assert_eq!((code, doc["status"].as_str()), (0, Some("ok")), "{doc}");
    assert!(diagnostics(&doc).is_empty(), "{doc}");

    let serial = fs::read_to_string(dir.join("out/traces/serial.csv")).unwrap();
    let rows: Vec<(u64, u8)> = serial
        .lines()
        .skip(1)
        .filter(|row| row.split(',').nth(2) == Some("USB_SERIAL_JTAG"))
        .map(|row| {
            let mut fields = row.split(',');
            let t_ps: u64 = fields.next().unwrap().parse().unwrap();
            let byte: u8 = fields.nth(2).unwrap().parse().unwrap();
            (t_ps, byte)
        })
        .collect();
    assert_eq!(rows.len(), 10_240);
    assert!(rows.iter().all(|&(_, b)| b == b'A'));

    let mut packet_times = Vec::new();
    for chunk in rows.as_chunks::<64>().0 {
        let t = chunk[0].0;
        assert!(chunk.iter().all(|&(at, _)| at == t));
        packet_times.push(t);
    }
    assert_eq!(packet_times.len(), 160);
    // Each packet takes at least 62.5 us (62,500,000 ps), so 160 packets finish at >= 10 ms.
    assert!(
        packet_times[0] >= 62_500_000,
        "first packet too early: {} ps",
        packet_times[0]
    );
    assert!(
        *packet_times.last().unwrap() >= 10_000_000_000,
        "10 kB burst finished too fast: {} ps",
        packet_times.last().unwrap()
    );
    for pair in packet_times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap >= 62_500_000,
            "packet gap {gap} ps < 62.5 us between {} and {}",
            pair[0],
            pair[1]
        );
    }
}

/// Replaces the blink LED load with one pull-down resistor of `ohms` from `GPIO20` to `gnd`.
fn netlist_with_gpio20_resistor(bytes: &[u8], stop_ps: u64, ohms: f64) -> serde_json::Value {
    let mut netlist = netlist_for(bytes, stop_ps);
    let step = stop_ps.min(100_000_000);
    netlist["analysis"]["transient"]["max_step_ps"] = serde_json::json!(step);
    netlist["analysis"]["transient"]["grid_ps"] = serde_json::json!(step);
    netlist["nets"] = serde_json::json!(["gnd", "gpio20", "vcc"]);
    netlist["parts"][1]["pins"] = serde_json::json!({
        "VCC": "vcc",
        "GND": "gnd",
        "GPIO20": "gpio20"
    });
    netlist["parts"].as_array_mut().unwrap().truncate(2);
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "r1",
            "kind": "resistor",
            "pins": { "a": "gpio20", "b": "gnd" },
            "resistance": { "value": ohms, "unit": "ohm" },
            "ratings": { "power": { "value": 0.25, "unit": "W" } }
        }));
    netlist
}

/// Initial value (`'0'` or `'1'`) and subsequent `(t_ps, char)` changes of `signal` in `vcd`.
fn vcd_signal(vcd: &str, signal: &str) -> (char, Vec<(u64, char)>) {
    let id = vcd
        .lines()
        .find_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            (parts.len() == 6 && parts[0] == "$var" && parts[4] == signal).then_some(parts[3])
        })
        .unwrap_or_else(|| panic!("no {signal} in VCD:\n{vcd}"));
    let mut initial = None;
    let mut changes = Vec::new();
    let mut in_dumpvars = false;
    let mut now = 0u64;
    for line in vcd.lines() {
        if line == "$dumpvars" {
            in_dumpvars = true;
        } else if in_dumpvars && line == "$end" {
            in_dumpvars = false;
        } else if let Some(rest) = line.strip_prefix('#') {
            now = rest.parse().unwrap();
        } else if let Some(level) = line.strip_suffix(id)
            && level.len() == 1
        {
            let ch = level.chars().next().unwrap();
            if in_dumpvars {
                initial = Some(ch);
            } else {
                changes.push((now, ch));
            }
        }
    }
    (initial.expect("initial value in $dumpvars"), changes)
}

/// GPIO20 starts pulled up through the USB PHY's 2.4 kohm D+ pull-up (so an external 10 kohm
/// pull-down sees ~2.66 V and `logic.vcd` starts high at 0 ps), and releases it when firmware
/// clears `USB_PAD_ENABLE` (falling back to Off or the 45 kohm weak IO_MUX pull-up) or overrides
/// `DP_PULLUP` off.
#[test]
fn gpio20_shows_usb_dp_pullup_until_firmware_takes_the_pin() {
    let run_case = |literals: &[u32], code: &[u8]| {
        let bytes = program(literals, code);
        let netlist = netlist_with_gpio20_resistor(&bytes, 1_000_000_000, 10_000.0);
        let dir = Scratch::new();
        let blobs = dir.join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(Sha256::of(&bytes).to_string()), &bytes).unwrap();
        let (exit, doc) = run(&dir, &netlist, &[]);
        assert_eq!((exit, doc["status"].as_str()), (0, Some("ok")), "{doc}");
        assert!(diagnostics(&doc).is_empty(), "{doc}");
        let obs: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join("out/observations.json")).unwrap()).unwrap();
        let v_op = obs["op"]["#net.gpio20"].as_f64().unwrap();
        let vcd = fs::read_to_string(dir.join("out/traces/logic.vcd")).unwrap();
        (v_op, vcd_signal(&vcd, "GPIO20"))
    };

    // Idle chip: USB_PAD_ENABLE=1, DP_PULLUP=1 -> 3.3 V * 10k / (2.4k + 10k) ~= 2.661 V.
    let (v_op, (init, edges)) = run_case(&[], &IDLE);
    assert!(
        (v_op - 3.3 * 10_000.0 / 12_400.0).abs() < 0.01,
        "expected ~2.661 V from 2.4k D+ pull-up into 10k load, got {v_op} V"
    );
    assert_eq!(init, '1', "GPIO20 starts high at 0 ps");
    assert!(edges.is_empty(), "stays high while idle: {edges:?}");

    // Clearing USB_PAD_ENABLE (CONF0 = 0x0200) releases the D+ pull-up so 10k pulls GPIO20 low.
    let (literals, code) = stores(&[(0x6003_8018, 0x0200)]);
    let (v_op, (init, edges)) = run_case(&literals, &code);
    assert!((v_op - 2.661).abs() < 0.01, "op {v_op} V");
    assert_eq!(init, '1');
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert_eq!(edges[0].1, '0', "clearing USB_PAD_ENABLE releases pull-up");

    // Taking GPIO20 with IO_MUX FUN_WPU (45k weak pull-up) drops the 10k load below VIL (~0.60 V).
    let (literals, code) = stores(&[
        (0x6000_9054, 1 << 12 | 1 << 9 | 1 << 8),
        (0x6003_8018, 0x0200),
    ]);
    let (v_op, (init, edges)) = run_case(&literals, &code);
    assert!((v_op - 2.661).abs() < 0.01, "op {v_op} V");
    assert_eq!(init, '1');
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert_eq!(edges[0].1, '0', "45k weak pull-up into 10k load reads low");

    // Overriding DP_PULLUP off (PAD_PULL_OVERRIDE=1, DP_PULLUP=0) while USB_PAD_ENABLE=1 drops it.
    let (literals, code) = stores(&[(0x6003_8018, 0x4100)]);
    let (v_op, (init, edges)) = run_case(&literals, &code);
    assert!((v_op - 2.661).abs() < 0.01, "op {v_op} V");
    assert_eq!(init, '1');
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert_eq!(edges[0].1, '0', "clearing DP_PULLUP releases pull-up");
}
