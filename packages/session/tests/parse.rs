//! The duration, condition and command parsers, as a test line meets them.

use std::path::Path;

use orbe_session::{
    Command, Condition, Duration, Needle, Op, Ref, Step, UNTIL_TIMEOUT, Unit, Want,
};

fn ps(text: &str) -> u64 {
    Duration::parse(text).unwrap().as_ps()
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn line(text: &str) -> Result<Option<Command>, String> {
    Command::parse(text, Path::new("/project"))
}

#[test]
fn durations_are_whole_picoseconds() {
    assert_eq!(ps("500ms"), 500_000_000_000);
    assert_eq!(ps("1.5ms"), 1_500_000_000);
    assert_eq!(ps("20us"), 20_000_000);
    assert_eq!(ps("20µs"), 20_000_000);
    assert_eq!(ps("3ns"), 3_000);
    assert_eq!(ps("7ps"), 7);
    assert_eq!(ps("2s"), 2_000_000_000_000);
    assert_eq!(ps("0.000000000001s"), 1);
    assert_eq!(ps("2.000ms"), 2_000_000_000);
}

#[test]
fn a_duration_refuses_what_is_not_whole_picoseconds() {
    for (text, says) in [
        ("1.5ps", "fraction of a picosecond"),
        ("1.0001ns", "fraction of a picosecond"),
        ("100", "has no unit"),
        ("10min", "not a time unit"),
        ("ms", "not a duration"),
        ("1.ms", "not a duration"),
        ("1.2.3ms", "not a duration"),
        ("99999999999s", "longer than a session can run"),
    ] {
        let error = Duration::parse(text).unwrap_err();
        assert!(error.contains(says), "{text}: {error}");
    }
}

#[test]
fn a_duration_prints_in_the_largest_unit_it_fills() {
    for (ps, text) in [
        (0, "0s"),
        (100_000_000_000, "100ms"),
        (100_004_062_500, "100.0040625ms"),
        (1_500_000_000_000, "1.5s"),
        (20_000_000, "20us"),
    ] {
        assert_eq!(Duration::from_ps(ps).to_string(), text);
        assert_eq!(
            Duration::parse(text).unwrap().as_ps(),
            ps,
            "{text} reads back"
        );
    }
}

#[test]
fn each_condition_form_parses() {
    let led = Ref::parse("@led1").unwrap();
    assert_eq!(
        Condition::parse(&words("@led1 on")).unwrap(),
        Condition::State(led, Want::On)
    );
    assert_eq!(
        Condition::parse(&words("@nano.D13 low")).unwrap(),
        Condition::State(Ref::parse("@nano.D13").unwrap(), Want::Low)
    );
    let Condition::Compare(r, op, amount) = Condition::parse(&words("@led1.i >= 5mA")).unwrap()
    else {
        panic!("not a comparison");
    };
    assert_eq!(
        (r.to_string(), op, amount.unit),
        ("@led1.i".into(), Op::Ge, Unit::Ampere)
    );
    assert!((amount.value - 0.005).abs() < 1e-15);
    let Condition::Compare(_, _, volts) = Condition::parse(&words("@nano.D2 < 2.5V")).unwrap()
    else {
        panic!("not a comparison");
    };
    assert_eq!((volts.value, volts.unit), (2.5, Unit::Volt));
    assert_eq!(
        Condition::parse(&["serial".into(), "contains".into(), "two words".into()]).unwrap(),
        Condition::SerialContains(Needle::parse("two words").unwrap())
    );
    assert_eq!(
        Condition::parse(&words("no diagnostics")).unwrap(),
        Condition::NoDiagnostics
    );
}

#[test]
fn an_amount_reads_the_numbers_a_reply_prints() {
    for (text, value) in [
        ("3.2687677014840023e-7A", 3.2687677014840023e-7),
        ("1e-3mA", 1e-6),
        ("2.5E+1V", 25.0),
        ("-0.0V", 0.0),
    ] {
        let Condition::Compare(_, _, amount) =
            Condition::parse(&words(&format!("@led1.i > {text}"))).unwrap()
        else {
            panic!("not a comparison");
        };
        assert_eq!(amount.value, value, "{text}");
    }
}

#[test]
fn a_bad_condition_says_what_to_write() {
    for (text, says) in [
        ("@led1 bright", "use on, off, high or low"),
        ("@led1.i == 5mA", "use <, <=, > or >="),
        ("@led1.i > 5", "not a value with a unit"),
        ("@led1.i > 5mOhm", "not a value with a unit"),
        ("@led1.i > 1e400A", "not a value with a unit"),
        ("@led1.i > 1e-A", "not a value with a unit"),
        ("led1 on", "not a condition"),
        ("@ on", "empty name"),
        ("@led1. on", "empty name"),
        ("", "missing"),
    ] {
        let error = Condition::parse(&words(text)).unwrap_err();
        assert!(error.contains(says), "{text}: {error}");
    }
    let error = line("expect serial contains \"\"").unwrap_err();
    assert!(error.contains("needs text"), "{error}");
}

#[test]
fn step_takes_a_duration_or_a_condition_with_a_timeout() {
    assert_eq!(
        line("step 100ms").unwrap(),
        Some(Command::Step(Step::For(Duration::from_ms(100))))
    );
    assert_eq!(
        line("step --until @led1 on --timeout 50ms").unwrap(),
        Some(Command::Step(Step::Until {
            condition: Condition::State(Ref::parse("@led1").unwrap(), Want::On),
            timeout: Duration::from_ms(50),
        }))
    );
    assert_eq!(
        line("step --until @led1.i > 5mA").unwrap(),
        Some(Command::Step(Step::Until {
            condition: Condition::parse(&words("@led1.i > 5mA")).unwrap(),
            timeout: UNTIL_TIMEOUT,
        }))
    );
    assert_eq!(UNTIL_TIMEOUT, Duration::parse("10s").unwrap());
}

#[test]
fn comments_blank_lines_and_quotes() {
    assert_eq!(line("").unwrap(), None);
    assert_eq!(line("   # a note").unwrap(), None);
    assert_eq!(line("close # done").unwrap(), Some(Command::Close));
    assert_eq!(
        line(r#"expect serial contains "a # \"b\" \\""#).unwrap(),
        Some(Command::Expect(Condition::SerialContains(
            Needle::parse(r#"a # "b" \"#).unwrap()
        )))
    );
    assert!(
        line(r#"expect serial contains "open"#)
            .unwrap_err()
            .contains("not closed")
    );
}

#[test]
fn open_reads_paths_from_the_project_folder() {
    assert_eq!(
        line("open").unwrap(),
        Some(Command::Open("/project/scene.jsonc".into()))
    );
    assert_eq!(
        line("open other.jsonc").unwrap(),
        Some(Command::Open("/project/other.jsonc".into()))
    );
}

#[test]
fn a_bad_line_names_the_command_and_its_usage() {
    for (text, says) in [
        (
            "blink @led1",
            "`blink` is not a command; the commands are open, close, press, release, get, step, play, pause, snapshot, expect",
        ),
        (
            "press",
            "takes one ref, such as `@btn1`; usage: press <ref>",
        ),
        ("press btn1", "`btn1` is not a ref"),
        ("close now", "takes no arguments; usage: close"),
        (
            "step",
            "usage: step <duration> | step --until <condition> [--timeout <duration>]",
        ),
        ("step 10", "has no unit"),
        ("open a b", "takes at most one scene"),
    ] {
        let error = line(text).unwrap_err();
        assert!(error.contains(says), "{text}: {error}");
    }
}

#[test]
fn the_wire_refuses_what_the_parser_refuses() {
    use serde_json::{Value, json};
    let refused = |wire: Value| serde_json::from_value::<Command>(wire).is_err();
    for text in ["btn1", "@", "@a..b", "@.a", "@a.", ""] {
        assert!(Ref::parse(text).is_err(), "{text}");
        assert!(refused(json!({ "Get": text })), "{text}");
        assert!(refused(json!({ "Press": text })), "{text}");
    }
    assert!(refused(json!({ "Get": [] })), "a ref with no name");
    assert!(refused(json!({ "Get": [""] })), "a ref with an empty name");
    for text in ["5", "mA", "5mX", "1e999A", "--5A", "5 mA"] {
        assert!(orbe_session::Amount::parse(text).is_err(), "{text}");
        assert!(
            refused(json!({ "Expect": { "Compare": ["@led1", "Gt", text] } })),
            "{text}"
        );
    }
    assert!(Condition::parse(&["serial".into(), "contains".into(), "".into()]).is_err());
    assert!(refused(json!({ "Expect": { "SerialContains": "" } })));
    assert!(refused(
        json!({ "Step": { "Until": { "condition": { "SerialContains": "" }, "timeout": 1 } } })
    ));

    for line in [
        "get @nano.D13",
        "press @btn1",
        "expect @led1.i > 5mA",
        "expect serial contains \"ready\"",
        "step --until @led1 on --timeout 50ms",
    ] {
        let command = line_ok(line);
        let wire = serde_json::to_value(&command).unwrap();
        assert_eq!(
            serde_json::from_value::<Command>(wire.clone()).unwrap(),
            command,
            "{wire}"
        );
    }
}

fn line_ok(text: &str) -> Command {
    line(text).unwrap().unwrap()
}
