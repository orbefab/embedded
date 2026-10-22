use sim_core::diagnostic::{
    AnalysisRule, Diagnostic, Excursion, ScheduleRule, Severity, Solve, ValueRule, WaveformRule,
};
use sim_core::name::{Name, NameError, Ref};
use sim_core::time::Time;

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
            found: 1,
            supported: &[0],
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
        }),
        Diagnostic::RatingPower(Excursion {
            quantity: "#r1.p".parse().unwrap(),
            limit: 0.25,
            peak: 3.0,
            peak_at: Time::from_ps(7),
            first: Time::from_ps(5),
            over_ps: Some(4),
        }),
        Diagnostic::NetlistSchedule {
            part: part("sw1"),
            field: "events",
            event: 1,
            rule: ScheduleRule::TooClose,
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
            field: "current.width_ps",
            value: 1500,
            rule: WaveformRule::TooClose,
        },
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
    assert_eq!(seen.len(), 22);
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
            "solver.convergence",
            "rating.current",
            "rating.power",
            "netlist.schedule",
            "netlist.analysis",
            "trace.too_large",
            "chip.unsettled",
            "netlist.waveform",
        ]
    );
    for diagnostic in &found {
        let code = diagnostic.code();
        let warning = code == "net.floating" || code.starts_with("rating.");
        assert_eq!(diagnostic.severity() == Severity::Warning, warning);
        if code.starts_with("solver.") || code == "rating.current" {
            assert_eq!(diagnostic.at(), Some(Time::ZERO));
        } else if code == "rating.power" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(5)));
        } else if code == "chip.unsettled" {
            assert_eq!(diagnostic.at(), Some(Time::from_ps(3_000)));
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
    let too_large = serde_json::to_string(by_code(&found, "solver.too_large")).unwrap();
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
            r##""message":"#led1.i peaks at 0.5 A, over its 0.03 A rating","data":{"limit":0.03,"over_ps":null,"peak":0.5,"peak_ps":0}"##
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
            r##""refs":["#r1.p"],"t_ps":5,"message":"#r1.p peaks at 3 W, over its 0.25 W rating","data":{"limit":0.25,"over_ps":4,"peak":3.0,"peak_ps":7}"##
        ),
        "{power}"
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
