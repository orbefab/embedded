//! What a command answers: the session's time and state, the command's data, and an error
//! with its fix when it failed. Text by default, one JSON envelope with `--json`.

use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::condition::Unit;
use crate::duration::Duration;

/// A reply's wire form, for a session process and its client, is its serde form. `json` is
/// the envelope a person or an agent reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub session: String,
    pub t_ps: u64,
    pub state: State,
    pub data: Data,
    pub error: Option<Error>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    /// No session is open.
    Closed,
    Paused,
    /// The session reached its 2-hour stop.
    Stopped,
    Faulted,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Closed => "closed",
            State::Paused => "paused",
            State::Stopped => "stopped",
            State::Faulted => "faulted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub code: Code,
    pub message: String,
    pub hint: String,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>, hint: impl Into<String>) -> Error {
        Error {
            code,
            message: message.into(),
            hint: hint.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    Usage,
    IoError,
    NoSession,
    SceneInvalid,
    PartMissing,
    FirmwareMissing,
    UnknownRef,
    /// The ref has no reading, level or unit the condition needs.
    NoReading,
    ExpectFailed,
    UntilTimeout,
    Faulted,
    /// The engine's `Rejected`.
    Ended,
    NotSettable,
    WrongSetting,
    Value,
    PastStop,
    NoChange,
    /// A step its caller stopped between two slices.
    Cancelled,
    /// The engine refused to open the netlist, with its diagnostic's code.
    Engine(String),
}

impl Code {
    pub fn as_str(&self) -> &str {
        match self {
            Code::Usage => "usage",
            Code::IoError => "io_error",
            Code::NoSession => "no_session",
            Code::SceneInvalid => "scene_invalid",
            Code::PartMissing => "part_missing",
            Code::FirmwareMissing => "firmware_missing",
            Code::UnknownRef => "unknown_ref",
            Code::NoReading => "no_reading",
            Code::ExpectFailed => "expect_failed",
            Code::UntilTimeout => "until_timeout",
            Code::Faulted => "faulted",
            Code::Ended => "ended",
            Code::NotSettable => "not_settable",
            Code::WrongSetting => "wrong_setting",
            Code::Value => "value",
            Code::PastStop => "past_stop",
            Code::NoChange => "no_change",
            Code::Cancelled => "cancelled",
            Code::Engine(code) => code,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Data {
    None,
    Opened {
        scene: String,
        parts: Vec<String>,
        engine_parts: usize,
        nets: usize,
        diagnostics: Vec<Diagnostic>,
    },
    /// The findings `orbe check` reports, for an `open` it refused.
    Invalid(Vec<Finding>),
    Closed {
        at_ps: u64,
    },
    Landed {
        target: String,
        pressed: bool,
        landed_ps: u64,
    },
    Got(Got),
    Stepped(Stepped),
    Snapshot(Snapshot),
    Checked(Check),
}

/// One `orbe check` finding, placed at its file, line and column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub hint: String,
}

impl Finding {
    pub fn new(d: orbe_scene::Diagnostic) -> Finding {
        Finding {
            code: d.code.as_str().to_string(),
            file: d.file,
            line: d.line,
            column: d.column,
            message: d.message,
            hint: d.hint,
        }
    }
}

/// `orbe check`'s own form: `scene.jsonc:4:23: error[part_missing]: ...` and its hint.
impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: error[{}]: {}\n  hint: {}",
            self.file, self.line, self.column, self.code, self.message, self.hint
        )
    }
}

/// An engine diagnostic: its code and the engine's JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub json: Value,
}

impl Diagnostic {
    pub fn new(diagnostic: &sim_core::diagnostic::Diagnostic) -> Diagnostic {
        Diagnostic {
            code: diagnostic.code().to_string(),
            json: serde_json::to_value(diagnostic).unwrap_or_default(),
        }
    }

    fn text(&self) -> String {
        let severity = self.json["severity"].as_str().unwrap_or("warning");
        let message = self.json["message"].as_str().unwrap_or_default();
        format!("{severity}[{}] {message}", self.code)
            .trim_end()
            .to_string()
    }

    /// Found while time ran, as opposed to a static warning `open` reports, which has no time.
    pub fn runtime(&self) -> bool {
        !self.json["t_ps"].is_null() || self.json["severity"] == "error"
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Got {
    pub target: String,
    pub value: Reading,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Reading {
    /// A pin: the chip's logic level when a chip pin shares its net, and its voltage.
    Pin {
        level: Option<Level>,
        v: f64,
    },
    Quantity {
        unit: Unit,
        value: f64,
    },
    /// A part: its `on` reading when it states one, and each engine quantity by name.
    Part {
        on: Option<bool>,
        quantities: Vec<(String, Unit, f64)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Low,
    High,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Low => "low",
            Level::High => "high",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stepped {
    pub from_ps: u64,
    pub met: Option<Met>,
    pub changes: Vec<Change>,
    /// Changes past the 20 a reply lists.
    pub more: usize,
    /// `--until`'s condition as it stood at the end.
    pub check: Option<Check>,
}

/// When `--until`'s condition first held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Met {
    /// An exact time: the step's start when it held there, a pin's edge, or the start bit of
    /// the byte that completed a serial match.
    At(u64),
    /// The slice end it was read at, for a condition read from the circuit, such as an `on`
    /// reading: it began to hold after the slice before and by then.
    By(u64),
}

/// What a step saw. An edge and serial carry the engine's own time, `at_ps`. An `on` reading
/// and a diagnostic are found by reading the circuit at a slice end, so they carry `by_ps`:
/// the change happened after the slice before and by then.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Edge {
        at_ps: u64,
        pin: String,
        level: Level,
    },
    Led {
        by_ps: u64,
        part: String,
        on: bool,
    },
    /// Text, at the start bit of the byte that completed it, or at the step's start for a
    /// byte that began before the step.
    Serial {
        at_ps: u64,
        text: String,
    },
    Diagnostic {
        by_ps: u64,
        diagnostic: Diagnostic,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub scene: String,
    pub parts: Vec<PartState>,
    pub serial: String,
    /// The bytes of a character still arriving, which `serial` leaves out.
    pub serial_arriving: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartState {
    pub name: String,
    pub ty: String,
    pub value: Option<Reading>,
    /// For a part with no engine part of its own, such as a board, its pins that share a net
    /// with another of the scene's parts.
    pub pins: Vec<(String, Reading)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub passed: bool,
    pub expected: String,
    pub actual: String,
}

fn t(ps: u64) -> String {
    Duration::from_ps(ps).to_string()
}

/// The shortest text that reads back as the same value: `0.012246558123229083`, `1.34e-13`.
pub(crate) fn number(value: f64) -> String {
    format!("{value:?}")
}

impl Reading {
    fn json(&self) -> Value {
        match self {
            Reading::Pin { level, v } => json!({ "level": level.map(Level::as_str), "v": v }),
            Reading::Quantity { value, .. } => json!(value),
            Reading::Part { on, quantities } => {
                let mut map = serde_json::Map::new();
                if let Some(on) = on {
                    map.insert("on".into(), json!(on));
                }
                for (name, _, value) in quantities {
                    map.insert(name.clone(), json!(value));
                }
                Value::Object(map)
            }
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Reading::Pin { .. } => "pin",
            Reading::Quantity { .. } => "quantity",
            Reading::Part { .. } => "part",
        }
    }

    /// A snapshot's short form: a pin's level, else its voltage, and a part's state and `i`.
    fn brief(&self) -> String {
        match self {
            Reading::Pin {
                level: Some(level), ..
            } => level.as_str().to_string(),
            Reading::Pin { level: None, v } => format!("{}V", number(*v)),
            Reading::Quantity { .. } => self.text(),
            Reading::Part { on, quantities } => Reading::Part {
                on: *on,
                quantities: quantities
                    .iter()
                    .filter(|(n, ..)| n == "i")
                    .cloned()
                    .collect(),
            }
            .text(),
        }
    }

    /// `high 4.62 V`, `0.0122 A`, `on i=0.0122 A v=1.93 V`.
    pub fn text(&self) -> String {
        match self {
            Reading::Pin {
                level: Some(level),
                v,
            } => format!("{} {} V", level.as_str(), number(*v)),
            Reading::Pin { level: None, v } => format!("{} V", number(*v)),
            Reading::Quantity { unit, value } => format!("{} {}", number(*value), unit.as_str()),
            Reading::Part { on, quantities } => {
                let mut words: Vec<String> = on
                    .map(|on| if on { "on" } else { "off" }.to_string())
                    .into_iter()
                    .collect();
                words.extend(
                    quantities
                        .iter()
                        .map(|(n, u, v)| format!("{n}={} {}", number(*v), u.as_str())),
                );
                words.join(" ")
            }
        }
    }
}

impl Check {
    fn json(&self) -> Value {
        json!({ "passed": self.passed, "expected": self.expected, "actual": self.actual })
    }
}

impl Change {
    /// When it happened, or the slice end it was found by.
    pub fn ps(&self) -> u64 {
        match self {
            Change::Edge { at_ps, .. } | Change::Serial { at_ps, .. } => *at_ps,
            Change::Led { by_ps, .. } | Change::Diagnostic { by_ps, .. } => *by_ps,
        }
    }

    fn json(&self) -> Value {
        match self {
            Change::Edge { at_ps, pin, level } => {
                json!({ "kind": "edge", "at_ps": at_ps, "ref": format!("@{pin}"), "level": level.as_str() })
            }
            Change::Led { by_ps, part, on } => {
                json!({ "kind": "led", "by_ps": by_ps, "ref": format!("@{part}"), "on": on })
            }
            Change::Serial { at_ps, text } => {
                json!({ "kind": "serial", "at_ps": at_ps, "text": text })
            }
            Change::Diagnostic { by_ps, diagnostic } => {
                json!({ "kind": "diagnostic", "by_ps": by_ps, "diagnostic": diagnostic.json })
            }
        }
    }

    fn text(&self) -> String {
        match self {
            Change::Edge { at_ps, pin, level } => format!("{} {pin} {}", t(*at_ps), level.as_str()),
            Change::Led { by_ps, part, on } => {
                format!("by {} {part} {}", t(*by_ps), if *on { "on" } else { "off" })
            }
            Change::Serial { at_ps, text } => format!("{} serial {text:?}", t(*at_ps)),
            Change::Diagnostic { by_ps, diagnostic } => {
                format!("by {} {}", t(*by_ps), diagnostic.text())
            }
        }
    }
}

impl Data {
    fn json(&self) -> Option<Value> {
        Some(match self {
            Data::None => return None,
            Data::Opened {
                scene,
                parts,
                engine_parts,
                nets,
                diagnostics,
            } => json!({
                "scene": scene,
                "parts": parts,
                "engine_parts": engine_parts,
                "nets": nets,
                "diagnostics": diagnostics.iter().map(|d| &d.json).collect::<Vec<_>>(),
            }),
            Data::Invalid(found) => json!({
                "diagnostics": found.iter().map(|d| json!({
                    "code": d.code.as_str(), "file": d.file, "line": d.line,
                    "column": d.column, "message": d.message, "hint": d.hint,
                })).collect::<Vec<_>>(),
            }),
            Data::Closed { at_ps } => json!({ "closed": true, "at_ps": at_ps }),
            Data::Landed {
                target, landed_ps, ..
            } => json!({ "ref": format!("@{target}"), "landed_ps": landed_ps }),
            Data::Got(got) => json!({
                "ref": format!("@{}", got.target),
                "kind": got.value.kind(),
                "value": got.value.json(),
            }),
            Data::Stepped(stepped) => {
                let (at, by) = match stepped.met {
                    Some(Met::At(ps)) => (Some(ps), None),
                    Some(Met::By(ps)) => (None, Some(ps)),
                    None => (None, None),
                };
                let mut value = json!({
                    "from_ps": stepped.from_ps,
                    "met_at_ps": at,
                    "met_by_ps": by,
                    "changes": stepped.changes.iter().map(Change::json).collect::<Vec<_>>(),
                    "more": stepped.more,
                });
                if let Some(check) = &stepped.check {
                    value["check"] = check.json();
                }
                value
            }
            Data::Snapshot(snapshot) => json!({
                "scene": snapshot.scene,
                "parts": snapshot.parts.iter().map(|p| json!({
                    "ref": format!("@{}", p.name),
                    "type": p.ty,
                    "value": p.value.as_ref().map(Reading::json),
                    "pins": p.pins.iter().map(|(pin, r)| (pin.clone(), r.json())).collect::<serde_json::Map<_, _>>(),
                })).collect::<Vec<_>>(),
                "serial": snapshot.serial,
                "serial_arriving": snapshot.serial_arriving,
                "diagnostics": snapshot.diagnostics.iter().map(|d| &d.json).collect::<Vec<_>>(),
            }),
            Data::Checked(check) => check.json(),
        })
    }

    fn text(&self, t_ps: u64) -> String {
        let mut out = String::new();
        match self {
            Data::None | Data::Invalid(_) => {}
            Data::Opened {
                scene,
                parts,
                engine_parts,
                nets,
                diagnostics,
            } => {
                let _ = write!(
                    out,
                    "opened {scene} at t=0: {} ({engine_parts} engine parts, {nets} nets)",
                    parts.join(", ")
                );
                for d in diagnostics {
                    let _ = write!(out, "\n  {}", d.text());
                }
            }
            Data::Closed { at_ps } => {
                let _ = write!(out, "closed at t={}", t(*at_ps));
            }
            Data::Landed {
                target,
                pressed,
                landed_ps,
            } => {
                let verb = if *pressed { "pressed" } else { "released" };
                let at = t(*landed_ps);
                let _ = write!(out, "{verb} {target} at t={at}; reads show it after t={at}");
            }
            Data::Got(got) => {
                let _ = write!(out, "{} {}", got.target, got.value.text());
            }
            Data::Stepped(stepped) => {
                let _ = write!(out, "t={} (from {})", t(t_ps), t(stepped.from_ps));
                if let Some(met) = stepped.met
                    && let Some(check) = &stepped.check
                {
                    let (word, ps) = match met {
                        Met::At(ps) => ("at", ps),
                        Met::By(ps) => ("by", ps),
                    };
                    let _ = write!(out, ", {} {word} {}", check.expected, t(ps));
                }
                for change in &stepped.changes {
                    let _ = write!(out, "\n  {}", change.text());
                }
                if stepped.more > 0 {
                    let _ = write!(out, "\n  and {} more", stepped.more);
                }
            }
            Data::Snapshot(snapshot) => {
                let _ = write!(out, "t={}  scene={}", t(t_ps), snapshot.scene);
                let width = snapshot
                    .parts
                    .iter()
                    .map(|p| p.name.len())
                    .max()
                    .unwrap_or(0);
                let ty_width = snapshot.parts.iter().map(|p| p.ty.len()).max().unwrap_or(0);
                for part in &snapshot.parts {
                    let mut words: Vec<String> = part.value.iter().map(Reading::brief).collect();
                    words.extend(
                        part.pins
                            .iter()
                            .map(|(pin, r)| format!("{pin}={}", r.brief())),
                    );
                    let _ = write!(
                        out,
                        "\n{:width$}  {:ty_width$}  {}",
                        part.name,
                        part.ty,
                        words.join("  ")
                    );
                }
                if !snapshot.serial.is_empty() || snapshot.serial_arriving > 0 {
                    let _ = write!(out, "\nserial  {:?}", snapshot.serial);
                }
                match snapshot.serial_arriving {
                    0 => {}
                    1 => out.push_str(" (1 byte still arriving)"),
                    n => {
                        let _ = write!(out, " ({n} bytes still arriving)");
                    }
                }
                if snapshot.diagnostics.is_empty() {
                    out.push_str("\ndiagnostics  none");
                }
                for d in &snapshot.diagnostics {
                    let _ = write!(out, "\n{}", d.text());
                }
            }
            Data::Checked(check) => {
                let _ = write!(out, "passed {} ({})", check.expected, check.actual);
            }
        }
        out
    }

    fn check(&self) -> Option<&Check> {
        match self {
            Data::Checked(check) => Some(check),
            Data::Stepped(stepped) => stepped.check.as_ref(),
            _ => None,
        }
    }
}

impl Reply {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    /// The `expected` and `actual` of a failed `expect` or `step --until`.
    pub fn check(&self) -> Option<&Check> {
        self.data.check()
    }

    pub fn json(&self) -> Value {
        let mut envelope = json!({
            "ok": self.ok(),
            "session": self.session,
            "t_ps": self.t_ps,
            "state": self.state.as_str(),
        });
        if let Some(data) = self.data.json() {
            envelope["data"] = data;
        }
        if let Some(error) = &self.error {
            envelope["error"] = json!({ "code": error.code.as_str(), "message": error.message, "hint": error.hint });
        }
        envelope
    }

    pub fn text(&self) -> String {
        let Some(error) = &self.error else {
            return self.data.text(self.t_ps);
        };
        let mut out = String::new();
        match &self.data {
            Data::Invalid(found) => {
                for d in found {
                    let _ = writeln!(out, "{d}");
                }
            }
            Data::Stepped(_) => {
                let _ = writeln!(out, "{}", self.data.text(self.t_ps));
            }
            _ => {}
        }
        let _ = write!(out, "error[{}]: {}", error.code.as_str(), error.message);
        if let Some(check) = self.check() {
            let _ = write!(
                out,
                "\n  expected: {}\n  actual:   {}",
                check.expected, check.actual
            );
        }
        if !error.hint.is_empty() {
            let _ = write!(out, "\n  hint: {}", error.hint);
        }
        out
    }
}
