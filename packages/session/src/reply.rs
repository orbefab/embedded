//! What a command answers: the session's time and state, the command's data, and an error
//! with its fix when it failed. Text by default, one JSON envelope with `--json`.

use std::fmt::{self, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use orbe_engine::world::Quantity;

use crate::condition::Unit;
use crate::duration::Duration;

/// A command's reply: the envelope `orbe <command> --json` prints. `data` is what the command
/// returned, and is absent for a command with nothing to return.
#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema), schemars(rename = "Reply"))]
pub(crate) struct Envelope<'a> {
    session: &'a str,
    t_ps: u64,
    state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(flatten)]
    outcome: Outcome<'a>,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(untagged)]
enum Outcome<'a> {
    Done {
        ok: Bool<true>,
    },
    Refused {
        ok: Bool<false>,
        error: ReplyError<'a>,
    },
}

/// A refused command's error, as the CLI prints it: `error[<code>]: <message>`, then the hint.
#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
struct ReplyError<'a> {
    code: &'a str,
    message: &'a str,
    /// Empty when there is none.
    hint: &'a str,
}

/// The JSON `true` or `false`, as a type, so an envelope's `ok` says which outcome it has.
struct Bool<const B: bool>;

impl<const B: bool> Serialize for Bool<B> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(B)
    }
}

#[cfg(test)]
impl<const B: bool> schemars::JsonSchema for Bool<B> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        B.to_string().into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "const": B })
    }
}

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

/// The session's state in a reply. `closed` is a session with no scene open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// No session is open.
    Closed,
    Paused,
    /// Time runs at real speed, since a `play`.
    Playing,
    /// The session reached its 2-hour stop.
    Stopped,
    Faulted,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Closed => "closed",
            State::Paused => "paused",
            State::Playing => "playing",
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

    /// The refusal for a path that is not a scene file. `open` and `check` both use it.
    pub fn missing_scene(scene: &Path) -> Error {
        Error::new(
            Code::IoError,
            format!("no scene file at {}", scene.display()),
            "write the scene, or name its path",
        )
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
    /// The session's process ended without closing.
    SessionCrashed,
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
            Code::SessionCrashed => "session_crashed",
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
    /// `play` or `pause`: whether time runs now.
    Paced {
        playing: bool,
    },
    /// A viewer's address, which `show` gives.
    Viewer {
        url: String,
    },
    Landed {
        target: String,
        pressed: bool,
        landed_ps: u64,
    },
    /// A world quantity's value from `landed_ps` on, in its unit. `already` is set when that
    /// value was already scheduled. It is left out of the JSON when false. When the scheduled
    /// value has not landed yet, `landed_ps` is that time.
    Set {
        quantity: Quantity,
        value: f64,
        landed_ps: u64,
        #[serde(default, skip_serializing_if = "is_false")]
        already: bool,
    },
    /// A world quantity as it stands at the session's time.
    World {
        quantity: Quantity,
        value: f64,
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
    pub fn new(diagnostic: &orbe_engine::diagnostic::Diagnostic) -> Diagnostic {
        Diagnostic {
            code: diagnostic.code().to_string(),
            json: serde_json::to_value(diagnostic).unwrap_or_default(),
        }
    }

    pub(crate) fn text(&self) -> String {
        let severity = self.json["severity"].as_str().unwrap_or("warning");
        let message = shown_message(&self.code, &self.json);
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

/// An LED's state from its average forward current. `lit` is above the part's `lit` reading,
/// `dim` above its `dim` reading, and `off` at or below that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Light {
    Lit,
    Dim,
    Off,
}

impl Light {
    pub fn as_str(self) -> &'static str {
        match self {
            Light::Lit => "lit",
            Light::Dim => "dim",
            Light::Off => "off",
        }
    }

    /// Strictly above each threshold, so a current exactly on one stays in the lower state.
    pub fn from_average(amperes: f64, lit: f64, dim: f64) -> Light {
        if amperes > lit {
            Light::Lit
        } else if amperes > dim {
            Light::Dim
        } else {
            Light::Off
        }
    }
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
    /// An LED's average forward current, and the span it covers.
    Brightness {
        value: f64,
        over_ps: u64,
    },
    /// A part: its light state and brightness when it states them, and each engine quantity.
    Part {
        state: Option<Light>,
        brightness: Option<(f64, u64)>,
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
    /// Each simulated LED at the end of the step, apart from `changes`, so a fade that stays
    /// in one state still shows its brightness.
    pub leds: Vec<LedRead>,
}

/// One LED at the end of a step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedRead {
    pub part: String,
    pub state: Light,
    pub brightness: f64,
    pub over_ps: u64,
}

/// When `--until`'s condition first held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Met {
    /// The step's start: the read there, the one `get` gives, held, and the step took no time.
    At(u64),
    /// A pin's edge. Reads show it from 1 ps later, where the step ends unless that is past
    /// its timeout or the stop.
    Edge(u64),
    /// The slice end it was read at, for a condition read from the circuit, such as a light
    /// state: it began to hold after the slice before and by then.
    By(u64),
    /// The slice end where the byte that completed the match first reached the session, and
    /// that byte's start bit.
    Serial { by_ps: u64, sent_at_ps: u64 },
}

/// What a step saw. An edge carries the engine's own time, `at_ps`. A light state, a
/// diagnostic and serial text are found at a slice end, so they carry `by_ps`: the change
/// happened after the slice before and by then.
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
        state: Light,
        brightness: f64,
        over_ps: u64,
    },
    /// Text, by the slice end it reached the session at, with the start bit of its last byte.
    Serial {
        by_ps: u64,
        sent_at_ps: u64,
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
    /// The bytes of a character held until the rest arrives, which `serial` leaves out.
    pub serial_held: usize,
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

/// The note for the bytes of a character the serial holds until the rest arrives.
pub(crate) fn held(bytes: usize) -> Option<String> {
    match bytes {
        0 => None,
        1 => Some("1 byte of a character held until the rest arrives".into()),
        n => Some(format!(
            "{n} bytes of a character held until the rest arrives"
        )),
    }
}

/// The shortest text that reads back as the same value: `0.012246558123229083`, `1.34e-13`.
pub(crate) fn number(value: f64) -> String {
    format!("{value:?}")
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A current in A, mA, µA or nA. The digits are the shortest decimal in that unit that
/// parses back to the same amperes. Under 1 nA stays in A, in scientific form. Zero is `0 A`.
pub(crate) fn current(amperes: f64) -> String {
    if !amperes.is_finite() {
        return format!("{amperes} A");
    }
    if amperes == 0.0 {
        return "0 A".to_string();
    }
    let sign = if amperes.is_sign_negative() { "-" } else { "" };
    let abs = amperes.abs();
    if abs < 1e-9 {
        return format!("{sign}{abs:?} A");
    }
    let (prefix, unit) = if abs >= 1.0 {
        (1.0, "A")
    } else if abs >= 1e-3 {
        (1e-3, "mA")
    } else if abs >= 1e-6 {
        (1e-6, "µA")
    } else {
        (1e-9, "nA")
    };
    format!("{sign}{} {unit}", shortest_scaled(abs, prefix))
}

fn quantity_text(unit: Unit, value: f64) -> String {
    match unit {
        Unit::Ampere => current(value),
        _ => format!("{} {}", number(value), unit.as_str()),
    }
}

/// The shortest decimal of `abs / prefix` whose parse, times `prefix`, is `abs`.
fn shortest_scaled(abs: f64, prefix: f64) -> String {
    let mut best: Option<String> = None;
    let mut consider = |text: &str| {
        let trimmed = trim_number(text);
        let candidate = if scales_back(&trimmed, abs, prefix) {
            trimmed
        } else if scales_back(text, abs, prefix) {
            text.to_string()
        } else {
            return;
        };
        if best.as_ref().is_none_or(|cur| shorter(&candidate, cur)) {
            best = Some(candidate);
        }
    };
    let quot = abs / prefix;
    if quot.is_finite() && quot > 0.0 {
        for prec in 0..=17 {
            consider(&format!("{quot:.prec$}"));
            consider(&format!("{quot:.prec$e}"));
        }
        consider(&format!("{quot:?}"));
        let bits = quot.to_bits();
        for delta in 1..=4096u64 {
            for step in [delta, 0u64.wrapping_sub(delta)] {
                let near = f64::from_bits(bits.wrapping_add(step));
                if near.is_finite() && near > 0.0 && near * prefix == abs {
                    consider(&format!("{near:?}"));
                }
            }
        }
    }
    best.unwrap_or_else(|| format!("{quot:?}"))
}

fn scales_back(text: &str, abs: f64, prefix: f64) -> bool {
    text.parse::<f64>()
        .ok()
        .is_some_and(|parsed| parsed.is_finite() && parsed > 0.0 && parsed * prefix == abs)
}

fn trim_number(text: &str) -> String {
    let (body, exp) = match text.find(['e', 'E']) {
        Some(at) => (&text[..at], Some(&text[at..])),
        None => (text, None),
    };
    let body = match body.split_once('.') {
        Some((whole, frac)) => {
            let frac = frac.trim_end_matches('0');
            if frac.is_empty() {
                whole.to_string()
            } else {
                format!("{whole}.{frac}")
            }
        }
        None => body.to_string(),
    };
    match exp {
        Some(exp) => format!("{body}{exp}"),
        None => body,
    }
}

/// Fewer characters, then a plain decimal ahead of scientific form.
fn shorter(new: &str, old: &str) -> bool {
    (new.len(), new.contains(['e', 'E']), new) < (old.len(), old.contains(['e', 'E']), old)
}

/// A rating message with its currents in [`current`] and a lasted `for … ps` in a duration.
fn shown_message(code: &str, json: &Value) -> String {
    let message = json["message"].as_str().unwrap_or_default();
    if code == "rating.current"
        && let Some(text) = rating_current(message, json)
    {
        return text;
    }
    if matches!(code, "rating.power" | "rating.voltage") {
        return with_over(message, json);
    }
    message.to_string()
}

fn with_over(message: &str, json: &Value) -> String {
    let Some(over) = json["data"]["over_ps"].as_u64().filter(|over| *over > 0) else {
        return message.to_string();
    };
    let suffix = format!(" for {over} ps");
    match message.strip_suffix(&suffix) {
        Some(head) => format!("{head} for {}", Duration::from_ps(over)),
        None => message.to_string(),
    }
}

fn rating_current(message: &str, json: &Value) -> Option<String> {
    let peak = json["data"]["peak"].as_f64()?;
    let limit = json["data"]["limit"].as_f64()?;
    let duration = json["data"]["over_ps"]
        .as_u64()
        .filter(|over| *over > 0)
        .map(|over| format!(" for {}", Duration::from_ps(over)))
        .unwrap_or_default();
    if let Some(window) = json["data"]["window_ps"].as_u64() {
        let head = message.split(" carries ").next()?;
        let ms = window as f64 / 1e9;
        Some(format!(
            "{head} carries {} RMS over the last {ms} ms, over its {} rating{duration}",
            current(peak),
            current(limit),
        ))
    } else {
        let head = message.split(" peaks at ").next()?;
        Some(format!(
            "{head} peaks at {}, over its {} rating{duration}",
            current(peak),
            current(limit),
        ))
    }
}

impl Reading {
    fn json(&self) -> Value {
        match self {
            Reading::Pin { level, v } => json!({ "level": level.map(Level::as_str), "v": v }),
            Reading::Quantity { value, .. } => json!(value),
            Reading::Brightness { value, over_ps } => json!({ "a": value, "over_ps": over_ps }),
            Reading::Part {
                state,
                brightness,
                quantities,
            } => {
                let mut map = serde_json::Map::new();
                if let Some(state) = state {
                    map.insert("state".into(), json!(state.as_str()));
                }
                if let Some((value, over_ps)) = brightness {
                    map.insert("brightness".into(), json!(value));
                    map.insert("over_ps".into(), json!(over_ps));
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
            Reading::Brightness { .. } => "brightness",
            Reading::Part { .. } => "part",
        }
    }

    /// A snapshot's short form: a pin's level, else its voltage, and a part's state, `i` and brightness.
    fn brief(&self) -> String {
        match self {
            Reading::Pin {
                level: Some(level), ..
            } => level.as_str().to_string(),
            Reading::Pin { level: None, v } => format!("{}V", number(*v)),
            Reading::Quantity { .. } | Reading::Brightness { .. } => self.text(),
            Reading::Part {
                state,
                brightness,
                quantities,
            } => Reading::Part {
                state: *state,
                brightness: *brightness,
                quantities: quantities
                    .iter()
                    .filter(|(n, ..)| n == "i")
                    .cloned()
                    .collect(),
            }
            .text(),
        }
    }

    /// `high 4.62 V`, `11.292504388060703 mA`, `lit i=11.292504388060703 mA v=1.93 V brightness=2.2562070877605347 mA over 20ms`.
    pub fn text(&self) -> String {
        match self {
            Reading::Pin {
                level: Some(level),
                v,
            } => format!("{} {} V", level.as_str(), number(*v)),
            Reading::Pin { level: None, v } => format!("{} V", number(*v)),
            Reading::Quantity { unit, value } => quantity_text(*unit, *value),
            Reading::Brightness { value, over_ps } => {
                format!("{} over {}", current(*value), t(*over_ps))
            }
            Reading::Part {
                state,
                brightness,
                quantities,
            } => {
                let mut words: Vec<String> = state
                    .map(|state| state.as_str().to_string())
                    .into_iter()
                    .collect();
                words.extend(
                    quantities
                        .iter()
                        .map(|(n, u, v)| format!("{n}={}", quantity_text(*u, *v))),
                );
                if let Some((value, over_ps)) = brightness {
                    words.push(format!(
                        "brightness={} over {}",
                        current(*value),
                        t(*over_ps)
                    ));
                }
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
            Change::Edge { at_ps, .. } => *at_ps,
            Change::Led { by_ps, .. }
            | Change::Diagnostic { by_ps, .. }
            | Change::Serial { by_ps, .. } => *by_ps,
        }
    }

    fn json(&self) -> Value {
        match self {
            Change::Edge { at_ps, pin, level } => {
                json!({ "kind": "edge", "at_ps": at_ps, "ref": format!("@{pin}"), "level": level.as_str() })
            }
            Change::Led {
                by_ps,
                part,
                state,
                brightness,
                over_ps,
            } => {
                json!({
                    "kind": "led",
                    "by_ps": by_ps,
                    "ref": format!("@{part}"),
                    "state": state.as_str(),
                    "brightness": brightness,
                    "over_ps": over_ps,
                })
            }
            Change::Serial {
                by_ps,
                sent_at_ps,
                text,
            } => {
                json!({ "kind": "serial", "by_ps": by_ps, "sent_at_ps": sent_at_ps, "text": text })
            }
            Change::Diagnostic { by_ps, diagnostic } => {
                json!({ "kind": "diagnostic", "by_ps": by_ps, "diagnostic": diagnostic.json })
            }
        }
    }

    fn text(&self) -> String {
        match self {
            Change::Edge { at_ps, pin, level } => format!("{} {pin} {}", t(*at_ps), level.as_str()),
            Change::Led {
                by_ps,
                part,
                state,
                brightness,
                over_ps,
            } => format!(
                "by {} {part} {}, brightness={} over {}",
                t(*by_ps),
                state.as_str(),
                current(*brightness),
                t(*over_ps)
            ),
            Change::Serial {
                by_ps,
                sent_at_ps,
                text,
            } => format!(
                "by {} serial {text:?} (last byte sent from {})",
                t(*by_ps),
                t(*sent_at_ps)
            ),
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
            Data::Paced { playing } => json!({ "playing": playing }),
            Data::Viewer { url } => json!({ "url": url }),
            Data::Landed {
                target, landed_ps, ..
            } => json!({ "ref": format!("@{target}"), "landed_ps": landed_ps }),
            Data::Set {
                quantity,
                value,
                landed_ps,
                already,
            } => {
                let mut body = json!({
                    "quantity": quantity.name(),
                    "value": value,
                    "unit": quantity.unit(),
                    "landed_ps": landed_ps,
                });
                if *already {
                    body["already"] = json!(true);
                }
                body
            }
            Data::World { quantity, value } => json!({
                "quantity": quantity.name(),
                "value": value,
                "unit": quantity.unit(),
            }),
            Data::Got(got) => json!({
                "ref": format!("@{}", got.target),
                "kind": got.value.kind(),
                "value": got.value.json(),
            }),
            Data::Stepped(stepped) => {
                let (at, by, sent) = match stepped.met {
                    Some(Met::At(ps) | Met::Edge(ps)) => (Some(ps), None, None),
                    Some(Met::By(ps)) => (None, Some(ps), None),
                    Some(Met::Serial { by_ps, sent_at_ps }) => {
                        (None, Some(by_ps), Some(sent_at_ps))
                    }
                    None => (None, None, None),
                };
                let mut value = json!({
                    "from_ps": stepped.from_ps,
                    "met_at_ps": at,
                    "met_by_ps": by,
                    "sent_at_ps": sent,
                    "changes": stepped.changes.iter().map(Change::json).collect::<Vec<_>>(),
                    "more": stepped.more,
                    "leds": stepped.leds.iter().map(|led| json!({
                        "ref": format!("@{}", led.part),
                        "state": led.state.as_str(),
                        "brightness": led.brightness,
                        "over_ps": led.over_ps,
                    })).collect::<Vec<_>>(),
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
                "serial_held": snapshot.serial_held,
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
                    "opened {scene} at t=0: {} ({}, {nets} nets)",
                    parts.join(", "),
                    orbe_scene::part_counts(parts.len(), *engine_parts)
                );
                for d in diagnostics {
                    let _ = write!(out, "\n  {}", d.text());
                }
            }
            Data::Closed { at_ps } => {
                let _ = write!(out, "closed at t={}", t(*at_ps));
            }
            Data::Paced { playing: true } => {
                let _ = write!(out, "playing from t={}", t(t_ps));
            }
            Data::Paced { playing: false } => {
                let _ = write!(out, "paused at t={}", t(t_ps));
            }
            Data::Viewer { url } => out.push_str(url),
            Data::Landed {
                target,
                pressed,
                landed_ps,
            } => {
                let verb = if *pressed { "pressed" } else { "released" };
                let at = t(*landed_ps);
                let _ = write!(out, "{verb} {target} at t={at}; reads show it after t={at}");
            }
            Data::Set {
                quantity,
                value,
                landed_ps,
                already,
            } => {
                if *already && *landed_ps > 0 && *landed_ps >= t_ps {
                    let _ = write!(
                        out,
                        "{} is already set to {value} {} from t={}",
                        quantity.name(),
                        quantity.unit(),
                        t(*landed_ps)
                    );
                } else if *already {
                    let _ = write!(
                        out,
                        "{} is already {value} {}",
                        quantity.name(),
                        quantity.unit()
                    );
                } else {
                    let at = t(*landed_ps);
                    let _ = write!(
                        out,
                        "set {} to {value} {} at t={at}; reads show it after t={at}",
                        quantity.name(),
                        quantity.unit()
                    );
                }
            }
            Data::World { quantity, value } => {
                let _ = write!(out, "{} {value} {}", quantity.name(), quantity.unit());
            }
            Data::Got(got) => {
                let _ = write!(out, "{} {}", got.target, got.value.text());
            }
            Data::Stepped(stepped) => {
                let _ = write!(out, "t={} (from {})", t(t_ps), t(stepped.from_ps));
                if let Some(met) = stepped.met
                    && let Some(check) = &stepped.check
                {
                    let _ = write!(out, ", {} ", check.expected);
                    let _ = match met {
                        Met::At(ps) => write!(out, "at {}", t(ps)),
                        Met::Edge(ps) if t_ps > ps => {
                            write!(out, "at {} (reads show it from {})", t(ps), t(t_ps))
                        }
                        Met::Edge(ps) => {
                            write!(out, "at {} (reads show it after {})", t(ps), t(ps))
                        }
                        Met::By(ps) => write!(out, "by {}", t(ps)),
                        Met::Serial { by_ps, sent_at_ps } => write!(
                            out,
                            "by {} (last byte sent from {})",
                            t(by_ps),
                            t(sent_at_ps)
                        ),
                    };
                }
                for change in &stepped.changes {
                    let _ = write!(out, "\n  {}", change.text());
                }
                if stepped.more > 0 {
                    let _ = write!(out, "\n  and {} more", stepped.more);
                }
                for led in &stepped.leds {
                    let _ = write!(
                        out,
                        "\n  brightness {} {} {} over {}",
                        led.part,
                        led.state.as_str(),
                        current(led.brightness),
                        t(led.over_ps)
                    );
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
                    let line = format!(
                        "{:width$}  {:ty_width$}  {}",
                        part.name,
                        part.ty,
                        words.join("  ")
                    );
                    let _ = write!(out, "\n{}", line.trim_end());
                }
                if !snapshot.serial.is_empty() || snapshot.serial_held > 0 {
                    let _ = write!(out, "\nserial  {:?}", snapshot.serial);
                }
                if let Some(held) = held(snapshot.serial_held) {
                    let _ = write!(out, " ({held})");
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

    /// `orbe check`'s exit codes: 0 when it succeeded, 2 for a usage error, 3 for an io error,
    /// and 1 for any other refusal.
    pub fn exit(&self) -> u8 {
        match self.error.as_ref().map(|e| &e.code) {
            None => 0,
            Some(Code::Usage) => 2,
            Some(Code::IoError) => 3,
            Some(_) => 1,
        }
    }

    /// The `expected` and `actual` of a failed `expect` or `step --until`.
    pub fn check(&self) -> Option<&Check> {
        self.data.check()
    }

    pub fn json(&self) -> Value {
        serde_json::to_value(self.envelope()).expect("an envelope serializes")
    }

    pub(crate) fn envelope(&self) -> Envelope<'_> {
        let outcome = match &self.error {
            None => Outcome::Done { ok: Bool },
            Some(error) => Outcome::Refused {
                ok: Bool,
                error: ReplyError {
                    code: error.code.as_str(),
                    message: &error.message,
                    hint: &error.hint,
                },
            },
        };
        Envelope {
            session: &self.session,
            t_ps: self.t_ps,
            state: self.state,
            data: self.data.json(),
            outcome,
        }
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
                // One finding whose code, message and hint are this error is already
                // printed above. A second line would repeat it.
                if let [one] = found.as_slice()
                    && one.code == error.code.as_str()
                    && one.message == error.message
                    && one.hint == error.hint
                {
                    out.pop();
                    return out;
                }
            }
            Data::Stepped(_) => {
                let _ = writeln!(out, "{}", self.data.text(self.t_ps));
            }
            _ => {}
        }
        let _ = write!(out, "error[{}]: {}", error.code.as_str(), error.message);
        if error.code == Code::ExpectFailed {
            let _ = write!(out, " at t={}", t(self.t_ps));
        }
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

#[cfg(test)]
mod tests {
    use super::current;
    use crate::condition::{Amount, Unit};

    #[test]
    fn a_printed_current_parses_back_to_the_same_amperes() {
        for amperes in [
            0.0,
            1.8,
            0.5,
            0.04,
            0.03,
            0.011292504388060703,
            0.004627811350286439,
            0.0030136985454632045,
            0.0016915818683638896,
            0.0022562070877605346,
            1.2775403302175985e-13,
            1.2775403302174304e-13,
            -0.00047583015647667537,
            0.06978612345678901,
        ] {
            let text = current(amperes);
            let parsed = Amount::parse(&text.replace(' ', ""))
                .unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(parsed.unit, Unit::Ampere, "{text}");
            assert_eq!(parsed.value, amperes, "{text}");
        }
    }
}
