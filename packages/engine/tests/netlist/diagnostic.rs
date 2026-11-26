use orbe_model::Time;

use orbe_engine::diagnostic::{
    AnalysisRule, CurrentLimitExcursion, Diagnostic, Excursion, LinkKind, ProtocolRule,
    ScheduleRule, Severity, Solve, SupplyExcursion, ValueRule, WaveformRule,
};
use orbe_engine::name::{Name, NameError, Ref};
use orbe_engine::netlist::{Average, Basis};
use orbe_mcu::{FlashRefusal, ResetReason, Setting};

fn name(text: &str) -> Name {
    Name::new(text).unwrap()
}

fn part(text: &str) -> Ref {
    Ref::Part(name(text))
}

fn samples() -> Vec<Diagnostic> {
    vec![
        Diagnostic::NetlistSyntax {
            path: "parts[0]".into(),
            line: 7,
            column: 21,
            detail: "unknown field `resistence`".into(),
        },
        Diagnostic::NetlistVersion {
            found: 2,
            supported: &[1],
        },
        Diagnostic::NetlistName {
            path: "parts[0].name".into(),
            name: "net".into(),
            problem: NameError::Reserved,
        },
        Diagnostic::NetlistDuplicate { name: part("r1") },
        Diagnostic::NetlistUnknownNet {
            pin: Ref::Pin(name("r2"), name("b")),
            net: "gnd2".into(),
        },
        Diagnostic::NetlistPins {
            part: part("r1"),
            missing: vec!["b"],
            unexpected: vec!["c".into()],
        },
        Diagnostic::NetlistUnit {
            part: part("r1"),
            field: "resistance".into(),
            expected: "ohm",
            found: "kohm".into(),
        },
        Diagnostic::NetlistValue {
            part: part("r1"),
            field: "resistance".into(),
            value: 0.0,
            rule: ValueRule::Positive,
        },
        Diagnostic::NetlistGround {
            net: "earth".into(),
        },
        Diagnostic::NetFloating {
            net: Ref::Net(name("isle")),
        },
        Diagnostic::SolverSingular {
            at: Time::ZERO,
            unknown: part("v2"),
        },
        Diagnostic::SolverNonFinite {
            at: Time::ZERO,
            unknown: part("r1"),
        },
        Diagnostic::SolverTooLarge {
            at: Time::ZERO,
            unknowns: 4097,
            limit: 4096,
        },
        Diagnostic::SolverStepLimit {
            at: Time::ZERO,
            limit: 100_000_000,
        },
        Diagnostic::SolverStepWork {
            at: Time::ZERO,
            limit: 1 << 30,
        },
        Diagnostic::SolverConvergence {
            at: Time::ZERO,
            unknown: part("led1"),
            iterations: 1310,
            gmin: None,
            source_scale: 0.75,
            solve: Solve::Ladder,
        },
        Diagnostic::RatingCurrent(Excursion {
            quantity: "#led1.i".parse().unwrap(),
            limit: 0.03,
            peak: 0.5,
            peak_at: Time::ZERO,
            first: Time::ZERO,
            over_ps: None,
            basis: Basis::Peak,
            sample: 0.5,
            held_ps: None,
        }),
        Diagnostic::RatingPower(Excursion {
            quantity: "#r1.p".parse().unwrap(),
            limit: 0.25,
            peak: 3.0,
            peak_at: Time::from_ps(7),
            first: Time::from_ps(5),
            over_ps: Some(4),
            basis: Basis::Sustained {
                tau_ps: 10_200_000_000,
                average: Average::Mean,
            },
            sample: 3.25,
            held_ps: None,
        }),
        Diagnostic::NetlistSchedule {
            part: part("sw1"),
            field: "events",
            event: 1,
            rule: ScheduleRule::TooClose,
            end_ps: 0,
        },
        Diagnostic::NetlistAnalysis {
            field: "max_step_ps",
            value: 999,
            rule: AnalysisRule::StepUnder1ns,
        },
        Diagnostic::TraceTooLarge {
            rows: 5,
            limit: 4,
            columns: 2,
            at: None,
        },
        Diagnostic::ChipUnsettled {
            at: Time::from_ps(3_000),
            chips: vec![part("u1"), part("u2")],
            rounds: 8,
        },
        Diagnostic::NetlistWaveform {
            part: part("i1"),
            field: "current.width_ps".into(),
            value: 1500,
            rule: WaveformRule::TooClose,
        },
        Diagnostic::ChipProtocol {
            part: part("lcd1"),
            at: Time::from_ps(4_000),
            rule: ProtocolRule::PixelFormat,
        },
        Diagnostic::ChipLinkMiswired {
            part: part("lcd1"),
            at: Time::from_ps(6_000),
            sender: name("rp1"),
        },
        Diagnostic::ChipLinkUnsupported {
            pin: Ref::Pin(name("u1"), name("GPIO4")),
            other: Ref::Pin(name("r1"), name("a")),
            at: Time::from_ps(7_000),
            peripheral: "RMT",
            kind: LinkKind::Pulses,
        },
        Diagnostic::RatingSupplyLow(SupplyExcursion {
            pin: Ref::Pin(name("u1"), name("VDD")),
            min: 1.71,
            clock_hz: None,
            lowest: 1.542,
            peak_at: Time::from_ps(9_500),
            first: Time::from_ps(8_500),
            over_ps: Some(2_000),
        }),
        Diagnostic::RegulatorCurrentLimit(CurrentLimitExcursion {
            pin: Ref::Pin(name("u1"), name("out")),
            limit: 0.03,
            target: 3.3,
            voltage: 3.003,
            current: 0.030007,
            peak_at: Time::from_ps(9_000),
            first: Time::from_ps(8_000),
            over_ps: Some(2_000),
        }),
    ]
}

fn by_code<'a>(found: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    found
        .iter()
        .find(|diagnostic| diagnostic.code() == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

fn is_code(code: &str) -> bool {
    let mut parts = code.split('.');
    let Some(head) = parts.next() else {
        return false;
    };
    if head.is_empty() || !head.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    let mut tails = 0;
    for part in parts {
        tails += 1;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return false;
        }
    }
    tails >= 1
}

#[test]
fn codes_are_unique_and_match_the_grammar() {
    let found = samples();
    let mut seen = std::collections::BTreeSet::new();
    for diagnostic in &found {
        let code = diagnostic.code();
        assert!(is_code(code), "{code}");
        assert!(seen.insert(code), "duplicate code {code}");
    }
    assert_eq!(seen.len(), 28);
}

/// `docs/engine/diagnostics.md` gives each code its own `## <code>` heading.
#[test]
fn each_code_has_its_heading_in_the_diagnostics_doc() {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/engine/diagnostics.md"),
    )
    .unwrap();
    for diagnostic in samples() {
        let heading = format!("\n## {}\n", diagnostic.code());
        assert!(doc.contains(&heading), "no heading {heading:?}");
    }
}

#[test]
fn severity_refs_and_time_follow_the_variant() {
    let found = samples();
    let codes: Vec<_> = found.iter().map(Diagnostic::code).collect();
    assert_eq!(
        codes,
        [
            "netlist.syntax",
            "netlist.version",
            "netlist.name",
            "netlist.duplicate",
            "netlist.unknown_net",
            "netlist.pins",
            "netlist.unit",
            "netlist.value",
            "netlist.ground",
            "net.floating",
            "solver.singular",
            "solver.nonfinite",
            "solver.too_large",
            "solver.step_limit",
            "solver.step_work",
            "solver.convergence",
            "rating.current",
            "rating.power",
            "netlist.schedule",
            "netlist.analysis",
            "trace.too_large",
            "chip.unsettled",
            "netlist.waveform",
            "chip.protocol",
            "chip.link_miswired",
            "chip.link_unsupported",
            "rating.supply_low",
            "regulator.current_limit",
        ]
    );
    for diagnostic in &found {
        let code = diagnostic.code();
        let warning = code == "net.floating"
            || code.starts_with("rating.")
            || code == "regulator.current_limit"
            || code == "chip.protocol"
            || code == "chip.link_miswired";
        assert_eq!(diagnostic.severity() == Severity::Warning, warning);
        if code.starts_with("solver.") || code == "rating.current" {
            assert_eq!(diagnostic.at(), Some(Time::ZERO));
        } else if code == "rating.power" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(5)));
        } else if code == "chip.unsettled" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(3_000)));
        } else if code == "chip.protocol" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(4_000)));
        } else if code == "chip.link_miswired" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(6_000)));
        } else if code == "chip.link_unsupported" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(7_000)));
        } else if code == "rating.supply_low" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(8_500)));
        } else if code == "regulator.current_limit" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(8_000)));
        } else {
            assert_eq!(diagnostic.at(), None);
        }
    }
    assert!(by_code(&found, "netlist.syntax").refs().is_empty());
    assert!(by_code(&found, "netlist.version").refs().is_empty());
    assert!(by_code(&found, "netlist.name").refs().is_empty());
    assert_eq!(
        by_code(&found, "netlist.duplicate").refs(),
        vec!["#r1".parse().unwrap()]
    );
    assert_eq!(
        by_code(&found, "netlist.unknown_net").refs(),
        vec!["#r2.pin.b".parse().unwrap()]
    );
    assert_eq!(
        by_code(&found, "netlist.pins").refs(),
        vec!["#r1".parse().unwrap()]
    );
    assert_eq!(
        by_code(&found, "netlist.unit").refs(),
        vec!["#r1".parse().unwrap()]
    );
    assert_eq!(
        by_code(&found, "netlist.value").refs(),
        vec!["#r1".parse().unwrap()]
    );
    assert!(by_code(&found, "netlist.ground").refs().is_empty());
    assert_eq!(
        by_code(&found, "net.floating").refs(),
        vec!["#net.isle".parse().unwrap()]
    );
    assert_eq!(
        by_code(&found, "solver.singular").refs(),
        vec!["#v2".parse().unwrap()]
    );
    assert_eq!(
        by_code(&found, "solver.nonfinite").refs(),
        vec!["#r1".parse().unwrap()]
    );
    assert!(by_code(&found, "solver.too_large").refs().is_empty());
    assert!(by_code(&found, "solver.step_limit").refs().is_empty());
    let step_limit = serde_json::to_string(by_code(&found, "solver.step_limit")).unwrap();
    assert!(
        step_limit.contains(
            r#""refs":[],"t_ps":0,"message":"the run accepted 100000000 steps, its limit; dense switch or source schedules cost steps, not rows","data":{"limit":100000000}"#
        ),
        "{step_limit}"
    );
    assert!(by_code(&found, "solver.step_work").refs().is_empty());
    let step_work = serde_json::to_string(by_code(&found, "solver.step_work")).unwrap();
    assert!(
        step_work.contains(
            r#""refs":[],"t_ps":0,"message":"one step took more than 1073741824 solver operations, its limit; the circuit is too large or too stiff to step","data":{"limit":1073741824}"#
        ),
        "{step_work}"
    );
    let too_large = serde_json::to_string(by_code(&found, "solver.too_large")).unwrap();
    assert!(
        too_large.contains(r#""message":"4097 unknowns exceeds the solver limit of 4096""#),
        "{too_large}"
    );
    assert!(
        too_large.contains(r#""data":{"limit":4096,"unknowns":4097}"#),
        "{too_large}"
    );
    assert_eq!(
        by_code(&found, "solver.convergence").refs(),
        vec!["#led1".parse().unwrap()]
    );
    let convergence = serde_json::to_string(by_code(&found, "solver.convergence")).unwrap();
    assert!(
        convergence.contains(r#""data":{"gmin":null,"iterations":1310,"source_scale":0.75}"#),
        "{convergence}"
    );
    assert_eq!(
        by_code(&found, "rating.current").refs(),
        vec!["#led1.i".parse().unwrap()]
    );
    let current = serde_json::to_string(by_code(&found, "rating.current")).unwrap();
    assert!(
        current.contains(
            r##""message":"#led1.i peaks at 0.5 A, over its 0.03 A rating","data":{"average":null,"held_ps":null,"limit":0.03,"now":false,"over_ps":null,"peak":0.5,"peak_ps":0,"sample":0.5,"tau_ps":null}"##
        ),
        "{current}"
    );
    let schedule = serde_json::to_string(by_code(&found, "netlist.schedule")).unwrap();
    assert!(
        schedule.contains(
            r##""refs":["#sw1"],"t_ps":null,"message":"#sw1 events[1] is less than 2000 ps after the previous event and its bounce window","data":{"event":1,"rule":"too_close"}"##
        ),
        "{schedule}"
    );
    let analysis = serde_json::to_string(by_code(&found, "netlist.analysis")).unwrap();
    assert!(
        analysis.contains(
            r#""refs":[],"t_ps":null,"message":"analysis max_step_ps is 999; it must be at least 1000","data":{"field":"max_step_ps","rule":"step_under_1ns","value":999}"#
        ),
        "{analysis}"
    );
    let rows = serde_json::to_string(by_code(&found, "trace.too_large")).unwrap();
    assert!(
        rows.contains(
            r#""severity":"error","refs":[],"t_ps":null,"message":"the grid gives 5 rows; the trace holds 4 rows of 2 columns; raise grid_ps or shorten stop_ps","data":{"columns":2,"limit":4,"rows":5}"#
        ),
        "{rows}"
    );
    let unsettled = serde_json::to_string(by_code(&found, "chip.unsettled")).unwrap();
    assert_eq!(
        unsettled,
        r##"{"code":"chip.unsettled","severity":"error","refs":["#u1","#u2"],"t_ps":3000,"message":"the chips kept changing state for 8 solves at 3000 ps","data":{"rounds":8}}"##
    );
    let protocol = serde_json::to_string(by_code(&found, "chip.protocol")).unwrap();
    assert_eq!(
        protocol,
        r##"{"code":"chip.protocol","severity":"warning","refs":["#lcd1"],"t_ps":4000,"message":"#lcd1 dropped a transfer that broke rule pixel_format at 4000 ps, the first time","data":{"rule":"pixel_format"}}"##
    );
    let miswired = serde_json::to_string(by_code(&found, "chip.link_miswired")).unwrap();
    assert_eq!(
        miswired,
        r##"{"code":"chip.link_miswired","severity":"warning","refs":["#lcd1"],"t_ps":6000,"message":"#lcd1 dropped an SPI transaction from #rp1 whose data lanes are not on its own data pins at 6000 ps, the first time","data":{"sender":"rp1"}}"##
    );
    let unsupported = serde_json::to_string(by_code(&found, "chip.link_unsupported")).unwrap();
    assert_eq!(
        unsupported,
        r##"{"code":"chip.link_unsupported","severity":"error","refs":["#u1.pin.GPIO4","#r1.pin.a"],"t_ps":7000,"message":"#u1.pin.GPIO4 sent a pulse record from RMT at 7000 ps, but its net reaches #r1.pin.a, which does not take pulse records there; the model sends RMT output as records rather than pin edges, so the data line must be wired straight to the pixel's data input","data":{"kind":"pulses","peripheral":"RMT"}}"##
    );
    let waveform = serde_json::to_string(by_code(&found, "netlist.waveform")).unwrap();
    assert!(
        waveform.contains(
            r##""refs":["#i1"],"t_ps":null,"message":"#i1 current.width_ps is 1500; two jumps must be at least 2000 ps apart","data":{"field":"current.width_ps","rule":"too_close","value":1500}"##
        ),
        "{waveform}"
    );
    let power = serde_json::to_string(by_code(&found, "rating.power")).unwrap();
    assert!(
        power.contains(
            r##""refs":["#r1.p"],"t_ps":5,"message":"#r1.p averages 3 W with a 10.2 ms time constant while it is 3.25 W, over its 0.25 W rating for 4 ps","data":{"average":"mean","held_ps":null,"limit":0.25,"now":false,"over_ps":4,"peak":3.0,"peak_ps":7,"sample":3.25,"tau_ps":10200000000}"##
        ),
        "{power}"
    );
    let supply_low = serde_json::to_string(by_code(&found, "rating.supply_low")).unwrap();
    assert_eq!(
        supply_low,
        r##"{"code":"rating.supply_low","severity":"warning","refs":["#u1.pin.VDD"],"t_ps":8500,"message":"#u1.pin.VDD supply drops to 1.54 V over the chip's ground, below its 1.71 V minimum for 2000 ps","data":{"limit":1.71,"over_ps":2000,"peak_ps":9500,"voltage":1.542}}"##
    );
    let reg_limit = serde_json::to_string(by_code(&found, "regulator.current_limit")).unwrap();
    assert_eq!(
        reg_limit,
        r##"{"code":"regulator.current_limit","severity":"warning","refs":["#u1.pin.out"],"t_ps":8000,"message":"#u1.pin.out regulator output can give 0.03 A at 3.3 V; the circuit asks for more, so it is giving 0.03 A and has dropped as low as 3 V for 2000 ps","data":{"current":0.030007,"limit":0.03,"over_ps":2000,"peak_ps":9000,"target":3.3,"voltage":3.003}}"##
    );
}

/// A sustained rating over now names its last sample, to 3 significant digits, and how long that
/// sample would hold before the filter crosses, to 2. Its `t_ps` is when the samples went over.
#[test]
fn a_rating_over_now_names_its_sample_and_how_long_it_would_hold() {
    let now = Diagnostic::RatingPower(Excursion {
        quantity: "#r1.p".parse().unwrap(),
        limit: 0.25,
        peak: 0.19393,
        peak_at: Time::from_ps(2_000_000_001),
        first: Time::from_ps(1),
        over_ps: Some(2_000_000_000),
        basis: Basis::Sustained {
            tau_ps: 10_200_000_000,
            average: Average::Mean,
        },
        sample: 0.9894505454260112,
        held_ps: std::num::NonZeroU64::new(578_337_123),
    });
    assert!(now.over_now());
    assert_eq!(
        serde_json::to_string(&now).unwrap(),
        r##"{"code":"rating.power","severity":"warning","refs":["#r1.p"],"t_ps":1,"message":"#r1.p is 0.989 W now, over its 0.25 W rating; it warns if held about 580000000 ps","data":{"average":"mean","held_ps":578337123,"limit":0.25,"now":true,"over_ps":2000000000,"peak":0.19393,"peak_ps":2000000001,"sample":0.9894505454260112,"tau_ps":10200000000}}"##
    );
}

#[test]
fn a_failed_restart_names_its_edge_and_keeps_the_operating_point_data() {
    let restart = Diagnostic::SolverConvergence {
        at: Time::from_ps(1_000_000),
        unknown: part("led1"),
        iterations: 12,
        gmin: None,
        source_scale: 0.0,
        solve: Solve::Ladder,
    };
    let text = serde_json::to_string(&restart).unwrap();
    assert!(
        text.contains(
            r##""t_ps":1000000,"message":"Newton did not converge at #led1 after 12 iterations in the restart solve at 1000000 ps; source stepping reached 0","data":{"gmin":null,"iterations":12,"source_scale":0.0}"##
        ),
        "{text}"
    );
}

#[test]
fn a_failed_time_step_adds_its_step_to_the_operating_point_data() {
    let step = Diagnostic::SolverConvergence {
        at: Time::from_ps(2_000_000_000),
        unknown: part("led1"),
        iterations: 50,
        gmin: None,
        source_scale: 1.0,
        solve: Solve::Step { step_ps: 1 },
    };
    let text = serde_json::to_string(&step).unwrap();
    assert!(
        text.contains(
            r##""t_ps":2000000000,"message":"Newton did not converge at #led1 after 50 iterations; the time step halved to 1 ps","data":{"gmin":null,"iterations":50,"source_scale":1.0,"step_ps":1}"##
        ),
        "{text}"
    );
}

#[test]
fn json_uses_the_catalog_field_order() {
    let found = samples();
    let unknown = by_code(&found, "netlist.unknown_net");
    let text = serde_json::to_string(unknown).unwrap();
    assert!(text.starts_with(
        r##"{"code":"netlist.unknown_net","severity":"error","refs":["#r2.pin.b"],"t_ps":null,"##
    ));
    assert!(
        text.contains(r##""message":"pin #r2.pin.b names net `gnd2`, which is not in `nets`""##)
    );
    assert!(text.ends_with(r#""data":{"net":"gnd2"}}"#));

    let unit = by_code(&found, "netlist.unit");
    let text = serde_json::to_string(unit).unwrap();
    assert!(text.contains(
        r##""message":"#r1 resistance is in `kohm`; it must be in `ohm` (SI base units)""##
    ));
    assert!(text.contains(r#""data":{"expected":"ohm","field":"resistance","found":"kohm"}"#));
}

#[test]
fn a_value_message_uses_the_same_characters_as_data_value() {
    for rule in [
        ValueRule::Positive,
        ValueRule::FiniteReciprocal,
        ValueRule::FiniteJunction,
        ValueRule::BounceRoom,
        ValueRule::NonNegative,
        ValueRule::BelowOne,
        ValueRule::AboveResistance,
        ValueRule::AboveDrop,
    ] {
        let diagnostic = Diagnostic::NetlistValue {
            part: part("d1"),
            field: "saturation_current".into(),
            value: 1e-308,
            rule,
        };
        let text = serde_json::to_string(&diagnostic).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let number = doc["data"]["value"].to_string();
        let message = doc["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("is {number};")),
            "{rule:?} {message} vs {number}"
        );
        assert!(!message.contains("000000"), "{rule:?} {message}");
    }
}

/// The message and `data` of an `esp32s3` refusal, as JSON.
fn chip_json(diagnostic: Diagnostic) -> (String, serde_json::Value) {
    let doc = serde_json::to_value(&diagnostic).unwrap();
    assert_eq!(doc["severity"], serde_json::json!(diagnostic.severity()));
    (
        doc["message"].as_str().unwrap().to_string(),
        doc["data"].clone(),
    )
}

#[test]
fn an_esp32s3_refusal_names_its_reason_in_its_message_and_data() {
    let at = Time::from_ps(5);
    let (message, data) = chip_json(Diagnostic::ChipFlashCommand {
        part: part("u1"),
        at,
        command: Some(0x31),
        reason: FlashRefusal::StatusWrite,
        pc: 0x4200_0010,
        app: false,
        symbol: None,
    });
    assert_eq!(
        message,
        "#u1 sent the flash the status write command 0x31 at 0x42000010, which could lock the flash, at 5 ps"
    );
    assert_eq!(
        data,
        serde_json::json!({ "command": 0x31, "pc": 0x4200_0010, "reason": "status_write", "symbol": null })
    );
    let (message, data) = chip_json(Diagnostic::ChipFlashCommand {
        part: part("u1"),
        at,
        command: Some(0x02),
        reason: FlashRefusal::Write,
        pc: 0x4200_0010,
        app: true,
        symbol: None,
    });
    assert_eq!(
        message,
        "#u1 wrote the flash with command 0x02 at 0x42000010 on the APP CPU, which the model does not simulate; Wi-Fi, Bluetooth and saved settings (Preferences, EEPROM, NVS) write the flash, at 5 ps"
    );
    assert_eq!(data["reason"], "write");
    let (message, data) = chip_json(Diagnostic::ChipFlashCommand {
        part: part("u1"),
        at,
        command: None,
        reason: FlashRefusal::StartBits,
        pc: 0x10,
        app: false,
        symbol: Some("main".into()),
    });
    assert_eq!(
        message,
        "#u1 started more than one SPI1 command in one write at 0x00000010 in main, which the model does not run, at 5 ps"
    );
    assert_eq!(data["reason"], "start_bits");
    let (message, data) = chip_json(Diagnostic::ChipFlashCommand {
        part: part("u1"),
        at,
        command: Some(0x03),
        reason: FlashRefusal::Cs,
        pc: 0x4200_0020,
        app: true,
        symbol: None,
    });
    assert_eq!(
        message,
        "#u1 sent the flash command 0x03 with CS0 disabled or CS1 enabled at 0x42000020 on the APP CPU, which the model does not run, at 5 ps"
    );
    assert_eq!(data["reason"], "cs");

    let setting = |setting, peripheral| Diagnostic::ChipUnsupportedSetting {
        part: part("u1"),
        at,
        peripheral,
        setting,
        pc: 0x10,
        app: false,
        symbol: None,
    };
    let (message, data) = chip_json(setting(Setting::Transfer("dma"), "SPI2"));
    assert_eq!(
        message,
        "#u1 started a SPI2 transfer with setting dma at 0x00000010, which the model does not run, at 5 ps"
    );
    assert_eq!(data["setting"], "dma");
    let (message, data) = chip_json(setting(Setting::Interrupt, "GDMA"));
    assert_eq!(
        message,
        "#u1 enabled a GDMA interrupt at 0x00000010, which the model never raises, at 5 ps"
    );
    assert_eq!(data["setting"], "interrupt");
    let (message, data) = chip_json(setting(Setting::Register("clock_off"), "GDMA"));
    assert_eq!(
        message,
        "#u1 set GDMA clock_off at 0x00000010, which the model does not run, at 5 ps"
    );
    assert_eq!(
        (&data["peripheral"], &data["setting"]),
        (&serde_json::json!("GDMA"), &serde_json::json!("clock_off"))
    );
    let reset = Setting::Register("reset_while_sending");
    let (message, data) = chip_json(setting(reset, "GDMA"));
    assert_eq!(
        message,
        "#u1 set GDMA reset_while_sending at 0x00000010, which the model does not run; ESP-IDF sets GDMA's reset when it restarts or panics, so the console holds any panic text, at 5 ps"
    );
    assert_eq!(data["setting"], "reset_while_sending");
    let (message, _) = chip_json(setting(reset, "SPI2"));
    assert_eq!(
        message,
        "#u1 set SPI2 reset_while_sending at 0x00000010, which the model does not run, at 5 ps"
    );
    let (message, data) = chip_json(Diagnostic::ChipHalted {
        part: part("u1"),
        at,
        pc: 0x10,
        app: false,
        symbol: Some("panic_abort+0x4".into()),
    });
    assert_eq!(
        message,
        "#u1 halted at 0x00000010 in panic_abort+0x4, a jump to itself that no interrupt or watchdog can leave, at 5 ps"
    );
    assert_eq!(
        data,
        serde_json::json!({ "pc": 0x10, "symbol": "panic_abort+0x4" })
    );
    let (message, data) = chip_json(Diagnostic::ChipHalted {
        part: part("u1"),
        at,
        pc: 0x10,
        app: true,
        symbol: None,
    });
    assert_eq!(
        message,
        "#u1 halted at 0x00000010 on the APP CPU, a jump to itself that no interrupt or watchdog can leave, at 5 ps"
    );
    assert_eq!(
        data,
        serde_json::json!({ "core": "app", "pc": 0x10, "symbol": null })
    );
    let (message, data) = chip_json(setting(
        Setting::Unsimulated("the pulse counter that encoder libraries use"),
        "PCNT",
    ));
    assert_eq!(
        message,
        "#u1 used PCNT, the pulse counter that encoder libraries use, at 0x00000010, which the model does not simulate, at 5 ps"
    );
    assert_eq!(
        (&data["peripheral"], &data["setting"]),
        (
            &serde_json::json!("PCNT"),
            &serde_json::json!("unsimulated")
        )
    );
    let (message, data) = chip_json(setting(Setting::Sleep, "RTC_CNTL"));
    assert_eq!(
        message,
        "#u1 put the chip to sleep at 0x00000010, which the model does not run, at 5 ps"
    );
    assert_eq!(data["peripheral"], "RTC_CNTL");
    let (message, data) = chip_json(setting(Setting::Register("pin_hold"), "RTC_CNTL"));
    assert_eq!(
        message,
        "#u1 enabled pin hold (gpio_hold_en) at 0x00000010, which the model does not run, at 5 ps"
    );
    assert_eq!(
        (&data["peripheral"], &data["setting"]),
        (
            &serde_json::json!("RTC_CNTL"),
            &serde_json::json!("pin_hold")
        )
    );

    let restart = |reason| Diagnostic::ChipFaultRestart {
        part: part("u1"),
        at,
        reason,
        pc: 0x10,
        app: false,
        symbol: None,
    };
    let (message, data) = chip_json(restart(ResetReason::Panic));
    assert_eq!(
        message,
        "#u1 reset itself at 0x00000010 after a panic, at 5 ps"
    );
    assert_eq!(
        data,
        serde_json::json!({ "pc": 0x10, "reason": "panic", "symbol": null })
    );
    let (message, data) = chip_json(restart(ResetReason::Unreadable));
    assert_eq!(
        message,
        "#u1 reset itself at 0x00000010 with no reset reason the model can read, at 5 ps"
    );
    assert_eq!(data["reason"], "unreadable");

    let (message, data) = chip_json(Diagnostic::ChipUnmappedLoad {
        part: part("u1"),
        at,
        address: 0x3c01_0000,
        pc: 0x10,
        app: false,
        symbol: None,
    });
    assert_eq!(
        message,
        "#u1 loaded from 0x3c010000 at 0x00000010, where the MMU maps no flash, at 5 ps"
    );
    assert_eq!(
        data,
        serde_json::json!({ "address": 0x3c01_0000, "pc": 0x10, "symbol": null })
    );

    let register = |kept| Diagnostic::ChipUnmodeledRegister {
        part: part("u1"),
        at,
        address: 0x600c_0020,
        register: "SYSTEM+0x20".into(),
        pc: 0x10,
        app: false,
        symbol: None,
        write: true,
        kept,
    };
    let (message, kept) = chip_json(register(true));
    let (dropped, data) = chip_json(register(false));
    assert_eq!(
        message,
        "#u1 wrote SYSTEM+0x20 at 0x600c0020, whose effect the model does not run, at 5 ps from 0x00000010"
    );
    assert_eq!(
        dropped,
        "#u1 wrote SYSTEM+0x20 at 0x600c0020, which the model does not keep, first at 5 ps from 0x00000010"
    );
    assert_eq!(
        data,
        serde_json::json!({
            "address": 0x600c_0020u32,
            "kept": false,
            "pc": 0x10,
            "register": "SYSTEM+0x20",
            "symbol": null,
            "write": true,
        })
    );
    assert_eq!(kept["kept"], true);

    let (message, data) = chip_json(Diagnostic::ChipUnmodeledRegister {
        part: part("u1"),
        at,
        address: 0x600c_0020,
        register: "SYSTEM+0x20".into(),
        pc: 0x10,
        app: true,
        symbol: None,
        write: false,
        kept: false,
    });
    assert_eq!(
        message,
        "#u1 read SYSTEM+0x20 at 0x600c0020, which the model does not keep, first at 5 ps from 0x00000010 on the APP CPU"
    );
    assert_eq!(data["core"], "app");
    let (message, data) = chip_json(Diagnostic::ChipUnmappedFetch {
        part: part("u1"),
        at,
        pc: 0x4000_0400,
        app: true,
        symbol: None,
    });
    assert_eq!(
        message,
        "#u1 fetched code at 0x40000400 on the APP CPU, where the model maps no memory, at 5 ps"
    );
    assert_eq!(
        data,
        serde_json::json!({ "core": "app", "pc": 0x4000_0400u32, "symbol": null })
    );
}
