//! What a command answers: the session's time and state, the command's data, and an error
//! with its fix when it failed. Text by default, one JSON envelope with `--json`.

use std::fmt::{self, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use orbe_engine::diagnostic::{significant_ps, significant_text, sustained_text, while_it_is};
use orbe_engine::netlist::Average;
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
    /// Set by the client when `open` starts again after a crash. Absent on the wire.
    #[serde(default, skip)]
    pub previous_crash: Option<String>,
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
    /// A step that reached its wall-time budget.
    StepTimeout,
    /// A step its caller stopped between two slices.
    Cancelled,
    /// The session's process ended without closing.
    SessionCrashed,
    /// A session another version of orbe started.
    VersionMismatch,
    /// The engine refused to open the netlist, with its diagnostic's code.
    Engine(String),
}

impl Code {
    /// The code `open` and `check` share for one scene's findings. `part_missing` or
    /// `firmware_missing` when every finding has that code, otherwise `scene_invalid`.
    /// No findings is `scene_invalid`.
    pub fn for_findings<'a>(codes: impl IntoIterator<Item = &'a str>) -> Self {
        let part = orbe_scene::Code::PartMissing.as_str();
        let firmware = orbe_scene::Code::FirmwareMissing.as_str();
        let mut found = None;
        for code in codes {
            match found {
                None => found = Some(code),
                Some(same) if same == code => {}
                Some(_) => return Code::SceneInvalid,
            }
        }
        match found {
            Some(code) if code == part => Code::PartMissing,
            Some(code) if code == firmware => Code::FirmwareMissing,
            _ => Code::SceneInvalid,
        }
    }

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
            Code::StepTimeout => "step_timeout",
            Code::Cancelled => "cancelled",
            Code::SessionCrashed => "session_crashed",
            Code::VersionMismatch => "version_mismatch",
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
    /// `play` or `pause`: whether time runs now. `pause` also carries what `play` held, when
    /// anything was. `from_ps` on that list is where the held stretch started.
    Paced {
        playing: bool,
        played: Option<Stepped>,
    },
    /// A viewer's address, which `show` gives. `viewers` is how many were already connected,
    /// so `open --headed` does not count the browser it is about to open.
    Viewer {
        url: String,
        viewers: usize,
    },
    Landed {
        target: String,
        pressed: bool,
        landed_ps: u64,
    },
    /// A world quantity's value from `landed_ps` on, in its unit. `already` is set when that
    /// value was already in force from the landing on, whether or not later steps were dropped.
    /// It is left out of the JSON when false. When that change has not landed yet, `landed_ps`
    /// is its time. `dropped` is how many later scheduled steps of this quantity the set
    /// removed, left out when none.
    Set {
        quantity: Quantity,
        value: f64,
        landed_ps: u64,
        #[serde(default, skip_serializing_if = "is_false")]
        already: bool,
        #[serde(default, skip_serializing_if = "is_zero")]
        dropped: u32,
    },
    /// A part's live param from `landed_ps` on, in the param's unit, such as `pot1.position`.
    /// `already` is as on `Set`.
    SetParam {
        target: String,
        value: f64,
        unit: String,
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
    /// The colours an addressable LED part last latched.
    Color(Colors),
    /// What a screen part's glass shows now.
    Screen(Screen),
    /// The two visible 16-character lines on a character LCD's glass.
    Text(LcdText),
    /// Whether a character LCD's backlight is lit.
    Backlight(bool),
    /// A live param's value, in the param's unit as the part file writes it, such as `1`.
    Param {
        unit: String,
        value: f64,
    },
    /// What a 7-segment part shows: `0` to `9`, `blank`, or `segments a,b`.
    Shown(String),
    /// A part: its light state and brightness when it states them, its colours when it latches
    /// them, its relay contact when it states one, and each engine quantity.
    Part {
        state: Option<Light>,
        brightness: Option<(f64, u64)>,
        color: Option<Colors>,
        /// `Some` when the part's contact reading is closed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contact: Option<bool>,
        quantities: Vec<(String, Unit, f64)>,
    },
}

/// What an addressable LED part last latched: one `[red, green, blue]` per LED, LED 0 first, and
/// when. `latched_ps` is `None` before the first latch or after power loss, while every LED shows
/// black.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colors {
    pub leds: Vec<[u8; 3]>,
    pub latched_ps: Option<u64>,
}

impl Colors {
    fn rgb([r, g, b]: [u8; 3]) -> String {
        format!("rgb({r},{g},{b})")
    }

    fn led_json([r, g, b]: [u8; 3]) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        map.insert("r".into(), json!(r));
        map.insert("g".into(), json!(g));
        map.insert("b".into(), json!(b));
        map
    }

    /// One LED's `r`, `g` and `b`, or a chain's `leds`, each with those, and `latched_ps`.
    fn json(&self) -> Value {
        let mut map = match self.leds.as_slice() {
            [one] => Colors::led_json(*one),
            leds => {
                let mut map = serde_json::Map::new();
                let leds: Vec<Value> = leds
                    .iter()
                    .map(|&rgb| Value::Object(Colors::led_json(rgb)))
                    .collect();
                map.insert("leds".into(), Value::Array(leds));
                map
            }
        };
        map.insert("latched_ps".into(), json!(self.latched_ps));
        Value::Object(map)
    }

    /// `rgb(255,0,0) latched at 12.5ms`, each LED's colour in chain order, or `rgb(0,0,0) not
    /// latched yet`.
    pub fn text(&self) -> String {
        let mut words: Vec<String> = self.leds.iter().map(|&rgb| Colors::rgb(rgb)).collect();
        words.push(match self.latched_ps {
            Some(ps) => format!("latched at {}", t(ps)),
            None => "not latched yet".to_string(),
        });
        words.join(" ")
    }
}

/// What a screen's glass shows: its controller kind, its size, whether it shows anything,
/// how many pixels are lit, and each row of pixels as text, `#` lit and `.` dark, top row
/// first. `settings` holds the controller's named settings, such as its contrast.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub kind: String,
    pub width: u16,
    pub height: u16,
    pub on: bool,
    pub lit: usize,
    pub rows: Vec<String>,
    pub settings: Vec<(String, u16)>,
}

impl Screen {
    fn json(&self) -> Value {
        json!({
            "kind": self.kind,
            "width": self.width,
            "height": self.height,
            "on": self.on,
            "lit": self.lit,
            "rows": self.rows,
            "settings": self.settings.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
        })
    }

    /// `ssd1306 128x64 on, 412 pixels lit`.
    pub fn brief(&self) -> String {
        format!(
            "{} {}x{} {}, {} pixels lit",
            self.kind,
            self.width,
            self.height,
            if self.on { "on" } else { "off" },
            self.lit
        )
    }

    /// The brief line, then each row.
    pub fn text(&self) -> String {
        let mut text = self.brief();
        for row in &self.rows {
            text.push('\n');
            text.push_str(row);
        }
        text
    }
}

/// The two visible 16-character lines on a character LCD's glass, after display shift.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcdText {
    pub lines: [String; 2],
    pub on: bool,
    pub backlight: bool,
}

impl LcdText {
    fn json(&self) -> Value {
        json!({
            "lines": self.lines,
            "on": self.on,
            "backlight": self.backlight,
        })
    }

    pub fn text(&self) -> String {
        format!("{}\n{}", self.lines[0], self.lines[1])
    }

    pub fn brief(&self) -> String {
        let a = self.lines[0].trim_end();
        let b = self.lines[1].trim_end();
        if a.is_empty() && b.is_empty() {
            "blank".to_string()
        } else {
            format!("{a:?} / {b:?}")
        }
    }
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
    /// Changes left out of `changes`. A `step` after `play` leaves out the overflow of each
    /// list, and each list stops at 20.
    pub more: usize,
    /// `--until`'s condition as it stood at the end.
    pub check: Option<Check>,
    /// Each simulated LED at the end of the step, apart from `changes`, so a fade that stays
    /// in one state still shows its brightness.
    pub leds: Vec<LedRead>,
}

impl Stepped {
    fn json(&self) -> Value {
        let (at, by, sent) = match self.met {
            Some(Met::At(ps) | Met::Edge(ps)) => (Some(ps), None, None),
            Some(Met::By(ps)) => (None, Some(ps), None),
            Some(Met::Serial { by_ps, sent_at_ps }) => (None, Some(by_ps), Some(sent_at_ps)),
            None => (None, None, None),
        };
        let mut value = json!({
            "from_ps": self.from_ps,
            "met_at_ps": at,
            "met_by_ps": by,
            "sent_at_ps": sent,
            "changes": self.changes.iter().map(Change::json).collect::<Vec<_>>(),
            "more": self.more,
            "leds": self.leds.iter().map(|led| json!({
                "ref": format!("@{}", led.part),
                "state": led.state.as_str(),
                "brightness": led.brightness,
                "over_ps": led.over_ps,
            })).collect::<Vec<_>>(),
        });
        if let Some(check) = &self.check {
            value["check"] = check.json();
        }
        value
    }

    fn write_text(&self, out: &mut String, t_ps: u64) {
        let _ = write!(out, "t={} (from {})", t(t_ps), t(self.from_ps));
        if let Some(met) = self.met
            && let Some(check) = &self.check
        {
            let _ = write!(out, ", {} ", check.expected);
            let _ = match met {
                Met::At(ps) => write!(out, "at {}", t(ps)),
                Met::Edge(ps) if t_ps > ps => {
                    write!(out, "at {} (reads show it from {})", t(ps), t(t_ps))
                }
                Met::Edge(ps) => write!(out, "at {} (reads show it after {})", t(ps), t(ps)),
                Met::By(ps) => write!(out, "by {}", t(ps)),
                Met::Serial { by_ps, sent_at_ps } => {
                    write!(
                        out,
                        "by {} (last byte sent from {})",
                        t(by_ps),
                        t(sent_at_ps)
                    )
                }
            };
        }
        for change in &self.changes {
            let _ = write!(out, "\n  {}", change.text());
        }
        if self.more > 0 {
            let _ = write!(out, "\n  and {} more", self.more);
        }
        for led in &self.leds {
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

/// A param's value with its unit, which a fraction, unit `1`, leaves out.
fn with_unit(value: f64, unit: &str) -> String {
    match unit {
        "1" | "" => number(value),
        unit => format!("{} {unit}", number(value)),
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u32) -> bool {
    *value == 0
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

/// `amperes` in the unit [`current`] picks, to 3 significant digits in that unit.
fn rated_current(amperes: f64) -> String {
    let abs = amperes.abs();
    let (prefix, unit) = if abs >= 1.0 || abs == 0.0 || !abs.is_finite() {
        (1.0, "A")
    } else if abs >= 1e-3 {
        (1e-3, "mA")
    } else if abs >= 1e-6 {
        (1e-6, "µA")
    } else {
        (1e-9, "nA")
    };
    let sign = if amperes.is_sign_negative() { "-" } else { "" };
    format!("{sign}{} {unit}", significant_text(abs / prefix))
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

/// A rating message as [`rating_message`] writes it, and a lasted `for … ps` in a duration.
fn shown_message(code: &str, json: &Value) -> String {
    let message = json["message"].as_str().unwrap_or_default();
    if matches!(code, "rating.current" | "rating.power")
        && let Some(text) = rating_message(code, message, json)
    {
        return text;
    }
    if code == "rating.voltage" {
        return with_over(message, json);
    }
    if code == "regulator.current_limit"
        && let Some(text) = regulator_limit_message(message, json)
    {
        return text;
    }
    message.to_string()
}

fn regulator_limit_message(message: &str, json: &Value) -> Option<String> {
    let data = &json["data"];
    let limit = data["limit"].as_f64()?;
    let target = data["target"].as_f64()?;
    let current = data["current"].as_f64()?;
    let voltage = data["voltage"].as_f64()?;
    let duration = data["over_ps"]
        .as_u64()
        .filter(|over| *over > 0)
        .map(|over| format!(" for {}", Duration::from_ps(over)))
        .unwrap_or_default();
    let end = message.find(" regulator output ")?;
    let head = &message[..end];
    Some(format!(
        "{head} regulator output can give {} at {} V; the circuit asks for more, so it is giving {} and has dropped as low as {} V{duration}",
        rated_current(limit),
        significant_text(target),
        rated_current(current),
        significant_text(voltage),
    ))
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

/// The engine's rating message with currents in [`current`], each value to 3 significant
/// digits, and times as durations. The head, the quantity as the message names it, is kept.
fn rating_message(code: &str, message: &str, json: &Value) -> Option<String> {
    let data = &json["data"];
    let value = |v: f64| {
        if code == "rating.current" {
            rated_current(v)
        } else {
            format!("{} W", significant_text(v))
        }
    };
    let peak = data["peak"].as_f64()?;
    let limit = data["limit"].as_f64()?;
    let sample = data["sample"].as_f64().unwrap_or(peak);
    let duration = data["over_ps"]
        .as_u64()
        .filter(|over| *over > 0)
        .map(|over| format!(" for {}", Duration::from_ps(over)))
        .unwrap_or_default();
    let rating = format!("over its {} rating{duration}", value(limit));
    let head = [" is ", " averages ", " carries ", " peaks at "]
        .iter()
        .filter_map(|verb| message.find(verb))
        .min()
        .map(|end| &message[..end])?;
    if let Some(held) = data["held_ps"].as_u64() {
        return Some(format!(
            "{head} is {} now, over its {} rating; it warns if held about {}",
            value(sample),
            value(limit),
            Duration::from_ps(significant_ps(held))
        ));
    }
    Some(match (data["tau_ps"].as_u64(), data["average"].as_str()) {
        (Some(tau), Some(average)) => {
            let average = if average == "rms" {
                Average::Rms
            } else {
                Average::Mean
            };
            format!(
                "{head} {} with a {} time constant{}, {rating}",
                sustained_text(average, value(peak)),
                Duration::from_ps(tau),
                while_it_is(value(sample), value(peak)),
            )
        }
        _ => format!("{head} peaks at {}, {rating}", value(peak)),
    })
}

impl Reading {
    fn json(&self) -> Value {
        match self {
            Reading::Pin { level, v } => json!({ "level": level.map(Level::as_str), "v": v }),
            Reading::Quantity { value, .. } => json!(value),
            Reading::Brightness { value, over_ps } => json!({ "a": value, "over_ps": over_ps }),
            Reading::Color(colors) => colors.json(),
            Reading::Screen(screen) => screen.json(),
            Reading::Text(lcd) => lcd.json(),
            Reading::Backlight(on) => json!({ "state": if *on { "lit" } else { "off" }, "on": on }),
            Reading::Shown(text) => json!({ "shown": text }),
            Reading::Param { value, .. } => json!(value),
            Reading::Part {
                state,
                brightness,
                color,
                contact,
                quantities,
            } => {
                let mut map = serde_json::Map::new();
                if let Some(closed) = contact {
                    map.insert(
                        "state".into(),
                        json!(if *closed { "closed" } else { "open" }),
                    );
                } else if let Some(state) = state {
                    map.insert("state".into(), json!(state.as_str()));
                }
                if let Some((value, over_ps)) = brightness {
                    map.insert("brightness".into(), json!(value));
                    map.insert("over_ps".into(), json!(over_ps));
                }
                if let Some(color) = color {
                    map.insert("color".into(), color.json());
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
            Reading::Color(_) => "color",
            Reading::Screen(_) => "screen",
            Reading::Text(_) => "text",
            Reading::Backlight(_) => "backlight",
            Reading::Shown(_) => "shown",
            Reading::Param { .. } => "param",
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
            Reading::Screen(screen) => screen.brief(),
            Reading::Text(lcd) => lcd.brief(),
            Reading::Backlight(on) => if *on { "lit" } else { "off" }.to_string(),
            Reading::Quantity { .. }
            | Reading::Brightness { .. }
            | Reading::Color(_)
            | Reading::Shown(_)
            | Reading::Param { .. } => self.text(),
            Reading::Part {
                state,
                brightness,
                color,
                contact,
                quantities,
            } => Reading::Part {
                state: *state,
                brightness: *brightness,
                color: color.clone(),
                contact: *contact,
                quantities: quantities
                    .iter()
                    .filter(|(n, u, _)| n == "i" || matches!(u, Unit::Degree | Unit::Hertz))
                    .cloned()
                    .collect(),
            }
            .text(),
        }
    }

    /// `high 4.62 V`, `11.292504388060703 mA`, `lit i=11.292504388060703 mA v=1.93 V brightness=2.2562070877605347 mA over 20ms`,
    /// `rgb(255,0,0) latched at 12.5ms`, `color=rgb(255,0,0) latched at 12.5ms`.
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
            Reading::Color(colors) => colors.text(),
            Reading::Screen(screen) => screen.text(),
            Reading::Text(lcd) => lcd.text(),
            Reading::Backlight(on) => if *on { "lit" } else { "off" }.to_string(),
            Reading::Shown(text) => text.clone(),
            Reading::Param { unit, value } => with_unit(*value, unit),
            Reading::Part {
                state,
                brightness,
                color,
                contact,
                quantities,
            } => {
                let mut words: Vec<String> = match contact {
                    Some(true) => vec!["closed".to_string()],
                    Some(false) => vec!["open".to_string()],
                    None => state
                        .map(|state| state.as_str().to_string())
                        .into_iter()
                        .collect(),
                };
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
                if let Some(color) = color {
                    words.push(format!("color={}", color.text()));
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
            Data::Paced { playing, played } => {
                let mut value = played
                    .as_ref()
                    .map(Stepped::json)
                    .unwrap_or_else(|| json!({}));
                value["playing"] = json!(playing);
                value
            }
            Data::Viewer { url, viewers } => json!({ "url": url, "viewers": viewers }),
            Data::Landed {
                target, landed_ps, ..
            } => json!({ "ref": format!("@{target}"), "landed_ps": landed_ps }),
            Data::Set {
                quantity,
                value,
                landed_ps,
                already,
                dropped,
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
                if *dropped > 0 {
                    body["dropped"] = json!(dropped);
                }
                body
            }
            Data::SetParam {
                target,
                value,
                unit,
                landed_ps,
                already,
            } => {
                let mut body = json!({
                    "ref": format!("@{target}"),
                    "value": value,
                    "unit": unit,
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
            Data::Stepped(stepped) => stepped.json(),
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
            Data::Paced {
                played: Some(stepped),
                ..
            } => stepped.write_text(&mut out, t_ps),
            Data::Paced {
                playing: true,
                played: None,
            } => {
                let _ = write!(out, "playing from t={}", t(t_ps));
            }
            Data::Paced {
                playing: false,
                played: None,
            } => {
                let _ = write!(out, "paused at t={}", t(t_ps));
            }
            Data::Viewer { url, .. } => out.push_str(url),
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
                dropped,
            } => {
                if *already {
                    let _ = write!(
                        out,
                        "{} is already {value} {}",
                        quantity.name(),
                        quantity.unit()
                    );
                    if *dropped > 0 {
                        let steps = if *dropped == 1 { "step" } else { "steps" };
                        let _ = write!(
                            out,
                            "; dropped {dropped} later scheduled {steps} of {}",
                            quantity.name()
                        );
                    }
                } else {
                    let at = t(*landed_ps);
                    let _ = write!(
                        out,
                        "set {} to {value} {} at t={at}",
                        quantity.name(),
                        quantity.unit()
                    );
                    if *dropped > 0 {
                        let steps = if *dropped == 1 { "step" } else { "steps" };
                        let _ = write!(
                            out,
                            "; dropped {dropped} later scheduled {steps} of {}",
                            quantity.name()
                        );
                    } else {
                        let _ = write!(out, "; reads show it after t={at}");
                    }
                }
            }
            Data::SetParam {
                target,
                value,
                unit,
                landed_ps,
                already,
            } => {
                let shown = with_unit(*value, unit);
                if *already {
                    let _ = write!(out, "{target} is already {shown}");
                } else {
                    let at = t(*landed_ps);
                    let _ = write!(
                        out,
                        "set {target} to {shown} at t={at}; reads show it after t={at}"
                    );
                }
            }
            Data::World { quantity, value } => {
                let _ = write!(out, "{} {value} {}", quantity.name(), quantity.unit());
            }
            Data::Got(got) => {
                let _ = write!(out, "{} {}", got.target, got.value.text());
            }
            Data::Stepped(stepped) => stepped.write_text(&mut out, t_ps),
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

    /// Exit codes: 0 when it succeeded, [`crate::EXIT_USAGE`] for a usage error,
    /// [`crate::EXIT_IO`] for an io error, [`crate::EXIT_LOST`] when the session's process
    /// is gone, and [`crate::EXIT_FAILED`] for any other refusal.
    pub fn exit(&self) -> u8 {
        match self.error.as_ref().map(|e| &e.code) {
            None => 0,
            Some(Code::Usage) => crate::EXIT_USAGE,
            Some(Code::IoError) => crate::EXIT_IO,
            Some(Code::SessionCrashed) => crate::EXIT_LOST,
            Some(_) => crate::EXIT_FAILED,
        }
    }

    /// The `expected` and `actual` of a failed `expect` or `step --until`.
    pub fn check(&self) -> Option<&Check> {
        self.data.check()
    }

    pub fn json(&self) -> Value {
        let mut value = serde_json::to_value(self.envelope()).expect("an envelope serializes");
        if let Some(note) = &self.previous_crash {
            let data = value
                .as_object_mut()
                .expect("an envelope is an object")
                .entry("data")
                .or_insert_with(|| json!({}));
            data.as_object_mut()
                .expect("data is an object")
                .insert("previous_crash".to_string(), note.clone().into());
        }
        value
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
            return with_crash(self.data.text(self.t_ps), self.previous_crash.as_deref());
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
                    return with_crash(out, self.previous_crash.as_deref());
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
        with_crash(out, self.previous_crash.as_deref())
    }
}

fn with_crash(mut text: String, note: Option<&str>) -> String {
    if let Some(note) = note {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(note);
    }
    text
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
