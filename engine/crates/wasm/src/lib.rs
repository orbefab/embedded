//! The session and `sim run` for JavaScript, through `wasm-bindgen`. `docs/wasm.md` is the guide.
//!
//! Times cross as `bigint` picoseconds, and `finish` and `run` take a `bigint` of unix seconds.
//! Per-frame data crosses as typed arrays. A diagnostic, the USART list and a display's state
//! cross as the JSON text the run folder writes, for the caller to parse. Every failure
//! the engine or the binding detects throws a `SimError`.

use sim_circuit::{Input, Refused, Rejected, Serial as SerialSend, Setting};
use sim_core::diagnostic::Diagnostic;
use sim_core::finite::Finite;
use sim_core::logic::Level;
use sim_core::name::Name;
use sim_core::netlist::{Baud, Contact, Cst816, Frame, Touch};
use sim_core::run_folder::RunFolder;
use sim_core::time::Time;
use sim_core::timestamp::Timestamp;
use wasm_bindgen::prelude::*;

/// Where a session stands after a call.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running = "running",
    Stopped = "stopped",
    Faulted = "faulted",
}

impl From<sim_circuit::Status> for Status {
    fn from(status: sim_circuit::Status) -> Status {
        match status {
            sim_circuit::Status::Running => Status::Running,
            sim_circuit::Status::Stopped => Status::Stopped,
            sim_circuit::Status::Faulted => Status::Faulted,
        }
    }
}

/// Why a call failed. The first three are why `open` or `record` made no session, the next seven
/// why an input changed nothing, then the binding's own checks of an argument's type, a part
/// name, a level, a baud and a `bigint`, a cursor before a dropped log entry, and `finish` on a
/// session `open` made. `display` and `displayImage` throw `unknown_part` for a part that is no
/// display part.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Op = "op",
    Failed = "failed",
    Invalid = "invalid",
    Ended = "ended",
    UnknownPart = "unknown_part",
    NotSettable = "not_settable",
    WrongSetting = "wrong_setting",
    Value = "value",
    PastStop = "past_stop",
    NoChange = "no_change",
    Type = "type",
    BadName = "bad_name",
    NotFinite = "not_finite",
    BadBaud = "bad_baud",
    Range = "range",
    Forgotten = "forgotten",
    Unrecorded = "unrecorded",
}

/// What every failing call throws.
#[wasm_bindgen(getter_with_clone)]
#[derive(Clone, Debug, PartialEq)]
pub struct SimError {
    #[wasm_bindgen(readonly)]
    pub code: Code,
    /// For `value`, the rule the level broke, as `diagnostics.json` names it, such as `fraction`.
    #[wasm_bindgen(readonly)]
    pub rule: Option<String>,
    /// A JSON array of the diagnostics behind `failed` or `invalid`, and `[]` for the rest.
    #[wasm_bindgen(readonly)]
    pub diagnostics: String,
}

impl SimError {
    fn new(code: Code) -> SimError {
        SimError {
            code,
            rule: None,
            diagnostics: "[]".to_string(),
        }
    }

    fn with(code: Code, diagnostics: &[Diagnostic]) -> SimError {
        SimError {
            diagnostics: json(diagnostics),
            ..SimError::new(code)
        }
    }
}

impl From<Refused> for SimError {
    fn from(refused: Refused) -> SimError {
        match refused {
            Refused::Op => SimError::new(Code::Op),
            Refused::Failed(diagnostic) => SimError::with(Code::Failed, &[diagnostic]),
            Refused::Invalid(diagnostics) => SimError::with(Code::Invalid, &diagnostics),
        }
    }
}

impl From<Rejected> for SimError {
    fn from(rejected: Rejected) -> SimError {
        let code = match rejected {
            Rejected::Ended => Code::Ended,
            Rejected::UnknownPart => Code::UnknownPart,
            Rejected::NotSettable => Code::NotSettable,
            Rejected::WrongSetting => Code::WrongSetting,
            Rejected::Value(_) => Code::Value,
            Rejected::PastStop => Code::PastStop,
            Rejected::NoChange => Code::NoChange,
        };
        let rule = match rejected {
            Rejected::Value(rule) => serde_json::to_value(rule)
                .ok()
                .and_then(|name| name.as_str().map(str::to_string)),
            _ => None,
        };
        SimError {
            rule,
            ..SimError::new(code)
        }
    }
}

/// The arguments as each target passes them. In wasm each is the JavaScript value, which the
/// binding checks for its type, because the glue would coerce a boolean or a number, reduce a
/// `bigint` modulo 2^64, read any object as bytes, and trap the module on a part or path that is
/// not a string. Natively each is the Rust value the adapter test passes, and only `name` and
/// `level` check it.
#[cfg(target_arch = "wasm32")]
mod arg {
    use super::{Code, SimError};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = Uint8Array)]
        type Bytes;

        // An exception from an import unwinds past the Rust frames and leaks the externref
        // slots they hold, and after 1024 every call traps, so a detached view must not throw.
        #[wasm_bindgen(method, catch)]
        fn subarray(this: &Bytes) -> Result<Vec<u8>, JsValue>;
    }

    pub type Netlist = JsValue;
    pub type Bigint = JsValue;
    pub type Text = JsValue;
    pub type Bool = JsValue;
    pub type Number = JsValue;

    fn wrong_type() -> SimError {
        SimError::new(Code::Type)
    }

    pub fn bytes_of(value: &Netlist) -> Result<Vec<u8>, SimError> {
        let bytes = value.dyn_ref::<Bytes>().ok_or_else(wrong_type)?;
        bytes.subarray().map_err(|_| wrong_type())
    }

    pub fn u64_of(value: Bigint) -> Result<u64, SimError> {
        if !value.is_bigint() {
            return Err(wrong_type());
        }
        u64::try_from(value).map_err(|_| SimError::new(Code::Range))
    }

    pub fn str_of(value: &Text) -> Result<String, SimError> {
        value.as_string().ok_or_else(wrong_type)
    }

    pub fn bool_of(value: Bool) -> Result<bool, SimError> {
        value.as_bool().ok_or_else(wrong_type)
    }

    pub fn f64_of(value: Number) -> Result<f64, SimError> {
        value.as_f64().ok_or_else(wrong_type)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod arg {
    use super::SimError;

    pub type Netlist = [u8];
    pub type Bigint = u64;
    pub type Text = str;
    pub type Bool = bool;
    pub type Number = f64;

    pub fn bytes_of(value: &Netlist) -> Result<Vec<u8>, SimError> {
        Ok(value.to_vec())
    }

    pub fn u64_of(value: Bigint) -> Result<u64, SimError> {
        Ok(value)
    }

    pub fn str_of(value: &Text) -> Result<String, SimError> {
        Ok(value.to_string())
    }

    pub fn bool_of(value: Bool) -> Result<bool, SimError> {
        Ok(value)
    }

    pub fn f64_of(value: Number) -> Result<f64, SimError> {
        Ok(value)
    }
}

use arg::{Bigint, Bool, Netlist, Number, Text, bool_of, bytes_of, f64_of, str_of, u64_of};

fn name(part: &str) -> Result<Name, SimError> {
    Name::new(part).map_err(|_| SimError::new(Code::BadName))
}

fn level(si: f64) -> Result<Finite, SimError> {
    Finite::new(si).ok_or(SimError::new(Code::NotFinite))
}

fn coordinate(value: f64) -> Result<u16, SimError> {
    let whole = value as u16;
    if value.is_finite() && f64::from(whole) == value && whole <= Cst816::COORD_MAX {
        return Ok(whole);
    }
    if !value.is_finite() {
        return Err(SimError::new(Code::NotFinite));
    }
    Err(SimError {
        rule: Some("span".to_string()),
        ..SimError::new(Code::Value)
    })
}

fn parse_baud(rate: f64) -> Result<Baud, SimError> {
    let whole = rate as u32;
    if rate.is_finite()
        && rate >= 1.0
        && f64::from(whole) == rate
        && let Some(baud) = Baud::new(whole)
    {
        return Ok(baud);
    }
    Err(SimError::new(Code::BadBaud))
}

fn json(value: &(impl serde::Serialize + ?Sized)) -> String {
    serde_json::to_string(value).expect("run folder JSON serializes")
}

/// Every analog column at `now`, in `refs()` order, and each column's integral from t = 0.
#[wasm_bindgen(getter_with_clone)]
pub struct Probe {
    #[wasm_bindgen(readonly)]
    pub at: u64,
    #[wasm_bindgen(readonly)]
    pub values: Vec<f64>,
    /// Both solves failed, so the values are on the line between the accepted steps around it.
    #[wasm_bindgen(readonly)]
    pub interpolated: bool,
    /// Each column's integral from t = 0 to `at`, in that column's unit times picoseconds.
    #[wasm_bindgen(readonly)]
    pub integrals: Vec<f64>,
}

/// Logic changes, one per index across the three arrays, and the cursor past them.
#[wasm_bindgen(getter_with_clone)]
pub struct Edges {
    /// The change time in picoseconds.
    #[wasm_bindgen(readonly)]
    pub at: Vec<u64>,
    /// An index into `signals()`.
    #[wasm_bindgen(readonly)]
    pub signal: Vec<u32>,
    /// 0 low, 1 high.
    #[wasm_bindgen(readonly)]
    pub level: Vec<u8>,
    #[wasm_bindgen(readonly)]
    pub cursor: u64,
}

/// Serial bytes, one per index across the three arrays, and the cursor past them.
#[wasm_bindgen(getter_with_clone)]
pub struct Serial {
    /// The start-bit time in picoseconds.
    #[wasm_bindgen(readonly)]
    pub at: Vec<u64>,
    /// An index into `usarts()`.
    #[wasm_bindgen(readonly)]
    pub usart: Vec<u16>,
    #[wasm_bindgen(readonly)]
    pub byte: Vec<u8>,
    #[wasm_bindgen(readonly)]
    pub cursor: u64,
}

/// Where an input landed, and the session's status after it.
#[wasm_bindgen]
pub struct Applied {
    #[wasm_bindgen(readonly)]
    pub at: u64,
    #[wasm_bindgen(readonly)]
    pub status: Status,
}

impl From<sim_circuit::Applied> for Applied {
    fn from(applied: sim_circuit::Applied) -> Applied {
        Applied {
            at: applied.at.as_ps(),
            status: applied.status.into(),
        }
    }
}

/// The files `sim run` writes, by their path in the folder.
#[wasm_bindgen]
pub struct Folder(RunFolder);

#[wasm_bindgen]
impl Folder {
    pub fn paths(&self) -> Vec<String> {
        self.0.files().map(|(path, _)| path.to_string()).collect()
    }

    pub fn file(
        &self,
        #[wasm_bindgen(unchecked_param_type = "string")] path: &Text,
    ) -> Result<Option<Vec<u8>>, SimError> {
        Ok(self.0.file(&str_of(path)?).map(<[u8]>::to_vec))
    }
}

/// A transient run held between calls.
#[wasm_bindgen]
pub struct Session {
    inner: sim_circuit::Session,
    refs: Vec<String>,
}

impl Session {
    fn new(inner: sim_circuit::Session) -> Session {
        let refs = inner.probe().refs().iter().map(|r| r.to_string()).collect();
        Session { inner, refs }
    }
}

/// A session opened from a document, so `export` has the JSON to write. A session
/// `Session::open` made from a `Netlist` has none, and is handed back.
impl TryFrom<sim_circuit::Session> for Session {
    type Error = sim_circuit::Session;

    fn try_from(inner: sim_circuit::Session) -> Result<Session, sim_circuit::Session> {
        if inner.has_document() {
            Ok(Session::new(inner))
        } else {
            Err(inner)
        }
    }
}

#[wasm_bindgen]
impl Session {
    /// A session on the netlist JSON that records nothing.
    pub fn open(
        #[wasm_bindgen(unchecked_param_type = "Uint8Array")] netlist: &Netlist,
    ) -> Result<Session, SimError> {
        let session = sim_circuit::Session::open_document(&bytes_of(netlist)?)?;
        Ok(Session::new(session))
    }

    /// A session that keeps what `sim run` writes, so `finish` returns its folder.
    pub fn record(
        #[wasm_bindgen(unchecked_param_type = "Uint8Array")] netlist: &Netlist,
    ) -> Result<Session, SimError> {
        let session = sim_circuit::Session::record_document(&bytes_of(netlist)?)?;
        Ok(Session::new(session))
    }

    #[wasm_bindgen(js_name = advanceTo)]
    pub fn advance_to(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "bigint")] ps: Bigint,
    ) -> Result<Status, SimError> {
        Ok(self.inner.advance_to(Time::from_ps(u64_of(ps)?)).into())
    }

    pub fn now(&self) -> u64 {
        self.inner.now().as_ps()
    }

    pub fn status(&self) -> Status {
        self.inner.status().into()
    }

    /// The diagnostic that ended a faulted session, as JSON.
    pub fn fault(&self) -> Option<String> {
        self.inner.fault().map(json)
    }

    /// The warnings `diagnostics.json` would list if the session ended now, as JSON.
    pub fn diagnostics(&self) -> String {
        json(&self.inner.diagnostics())
    }

    /// The analog columns `probe()` reads, in ref grammar.
    pub fn refs(&self) -> Vec<String> {
        self.refs.clone()
    }

    pub fn probe(&self) -> Probe {
        let probe = self.inner.probe();
        Probe {
            at: probe.at().as_ps(),
            values: probe.values().iter().map(|v| v.get()).collect(),
            interpolated: probe.interpolated(),
            integrals: probe.integrals().to_vec(),
        }
    }

    /// Each chip signal, in ref grammar.
    pub fn signals(&self) -> Vec<String> {
        self.inner.signals().iter().map(|r| r.to_string()).collect()
    }

    /// Each chip signal's level at `now`, in `signals()` order, 0 low and 1 high.
    pub fn levels(&self) -> Vec<u8> {
        self.inner
            .levels()
            .iter()
            .map(|level| u8::from(*level == Level::High))
            .collect()
    }

    /// Each USART as a JSON array of `[part, usart]` pairs, such as `[["u1","USART0"]]`.
    pub fn usarts(&self) -> String {
        let usarts: Vec<[&str; 2]> = self
            .inner
            .usarts()
            .iter()
            .map(|(part, usart)| [part.as_str(), usart])
            .collect();
        json(&usarts)
    }

    /// The bytes after the first `cursor` the session sent, and the cursor past them.
    #[wasm_bindgen(js_name = serialSince)]
    pub fn serial_since(
        &self,
        #[wasm_bindgen(unchecked_param_type = "bigint")] cursor: Bigint,
    ) -> Result<Serial, SimError> {
        let (bytes, next) = self
            .inner
            .serial_since(u64_of(cursor)?.into())
            .ok_or_else(|| SimError::new(Code::Forgotten))?;
        Ok(Serial {
            at: bytes.iter().map(|b| b.at.as_ps()).collect(),
            usart: bytes.iter().map(|b| b.usart).collect(),
            byte: bytes.iter().map(|b| b.byte).collect(),
            cursor: next.into(),
        })
    }

    /// Drops the serial bytes before `cursor`. Cursors keep counting from the first byte.
    #[wasm_bindgen(js_name = forgetSerial)]
    pub fn forget_serial(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "bigint")] cursor: Bigint,
    ) -> Result<(), SimError> {
        self.inner.forget_serial(u64_of(cursor)?.into());
        Ok(())
    }

    /// The cursor past every chip-signal change so far.
    #[wasm_bindgen(js_name = edgeCursor)]
    pub fn edge_cursor(&self) -> u64 {
        self.inner.edge_cursor().into()
    }

    /// The chip-signal changes after the first `cursor`, and the cursor past them.
    #[wasm_bindgen(js_name = edgesSince)]
    pub fn edges_since(
        &self,
        #[wasm_bindgen(unchecked_param_type = "bigint")] cursor: Bigint,
    ) -> Result<Edges, SimError> {
        let (changes, next) = self
            .inner
            .edges_since(u64_of(cursor)?.into())
            .ok_or_else(|| SimError::new(Code::Forgotten))?;
        Ok(Edges {
            at: changes.iter().map(|change| change.at.as_ps()).collect(),
            signal: changes
                .iter()
                .map(|change| u32::try_from(change.signal).expect("a signal index fits u32"))
                .collect(),
            level: changes
                .iter()
                .map(|change| u8::from(change.level == Level::High))
                .collect(),
            cursor: next.into(),
        })
    }

    /// Drops the chip-signal changes before `cursor`. Cursors keep counting from the first change.
    #[wasm_bindgen(js_name = forgetEdges)]
    pub fn forget_edges(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "bigint")] cursor: Bigint,
    ) -> Result<(), SimError> {
        self.inner.forget_edges(u64_of(cursor)?.into());
        Ok(())
    }

    /// The display part `part` at `now`, as the JSON of its `observations.json` `displays` entry
    /// without `image` and `sha256`.
    pub fn display(
        &self,
        #[wasm_bindgen(unchecked_param_type = "string")] part: &Text,
    ) -> Result<String, SimError> {
        Ok(json(&self.panel(part)?.panel()))
    }

    /// The display part `part`'s frame memory at `now`, as `traces/display-<part>.rgb565` holds
    /// it.
    #[wasm_bindgen(js_name = displayImage)]
    pub fn display_image(
        &self,
        #[wasm_bindgen(unchecked_param_type = "string")] part: &Text,
    ) -> Result<Vec<u8>, SimError> {
        Ok(self.panel(part)?.image())
    }

    fn panel(&self, part: &Text) -> Result<sim_circuit::Display<'_>, SimError> {
        self.inner
            .display(&name(&str_of(part)?)?)
            .ok_or(SimError::new(Code::UnknownPart))
    }

    /// Sets a switch's contact from `ps` on.
    #[wasm_bindgen(js_name = applyContact)]
    pub fn apply_contact(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "string")] part: &Text,
        #[wasm_bindgen(unchecked_param_type = "bigint")] ps: Bigint,
        #[wasm_bindgen(unchecked_param_type = "boolean")] closed: Bool,
    ) -> Result<Applied, SimError> {
        let (part, at) = (name(&str_of(part)?)?, u64_of(ps)?);
        let contact = if bool_of(closed)? {
            Contact::Closed
        } else {
            Contact::Open
        };
        self.apply(part, at, Setting::Contact(contact))
    }

    /// Sets a plain or `steps` voltage or current source, or a potentiometer, from `ps` on,
    /// in volts, amperes or the position fraction. A plain source is exported as a `steps`
    /// waveform.
    #[wasm_bindgen(js_name = applyLevel)]
    pub fn apply_level(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "string")] part: &Text,
        #[wasm_bindgen(unchecked_param_type = "bigint")] ps: Bigint,
        #[wasm_bindgen(unchecked_param_type = "number")] si: Number,
    ) -> Result<Applied, SimError> {
        let (part, at) = (name(&str_of(part)?)?, u64_of(ps)?);
        self.apply(part, at, Setting::Level(level(f64_of(si)?)?))
    }

    /// Sends 8N1 frames on a `serial` source from `ps` on, at `baud`.
    /// `bytes` is copied before this returns.
    #[wasm_bindgen(js_name = applySerial)]
    pub fn apply_serial(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "string")] part: &Text,
        #[wasm_bindgen(unchecked_param_type = "bigint")] ps: Bigint,
        #[wasm_bindgen(unchecked_param_type = "number")] baud: Number,
        #[wasm_bindgen(unchecked_param_type = "Uint8Array")] bytes: &Netlist,
    ) -> Result<Applied, SimError> {
        let part = name(&str_of(part)?)?;
        let at = u64_of(ps)?;
        let baud = parse_baud(f64_of(baud)?)?;
        let bytes = bytes_of(bytes)?;
        self.apply(
            part,
            at,
            Setting::Serial(SerialSend {
                baud,
                frame: Frame::EightN1,
                bytes,
            }),
        )
    }

    /// A point on a `cst816` from `ps` on. `x` and `y` are integers from 0 to 4095.
    #[wasm_bindgen(js_name = applyTouch)]
    pub fn apply_touch(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "string")] part: &Text,
        #[wasm_bindgen(unchecked_param_type = "bigint")] ps: Bigint,
        #[wasm_bindgen(unchecked_param_type = "number")] x: Number,
        #[wasm_bindgen(unchecked_param_type = "number")] y: Number,
        #[wasm_bindgen(unchecked_param_type = "boolean")] down: Bool,
    ) -> Result<Applied, SimError> {
        let part = name(&str_of(part)?)?;
        let at = u64_of(ps)?;
        let touch = Touch {
            x: coordinate(f64_of(x)?)?,
            y: coordinate(f64_of(y)?)?,
            down: bool_of(down)?,
        };
        self.apply(part, at, Setting::Touch(touch))
    }

    fn apply(&mut self, part: Name, at: u64, setting: Setting) -> Result<Applied, SimError> {
        let input = Input {
            part,
            at: Time::from_ps(at),
            setting,
        };
        Ok(self.inner.apply(&input)?.into())
    }

    /// The netlist JSON the session runs, with every applied input in its part. Every `Session`
    /// holds a document, so it is never empty.
    pub fn export(&self) -> Vec<u8> {
        self.inner.export().unwrap_or_default()
    }

    /// Advances to the stop and returns the folder `sim run` writes for the netlist the session
    /// ran, stamped `created_at_seconds` after the unix epoch. It frees the session, also when it
    /// throws.
    pub fn finish(
        self,
        #[wasm_bindgen(js_name = createdAtSeconds, unchecked_param_type = "bigint")]
        created_at_seconds: Bigint,
    ) -> Result<Folder, SimError> {
        let created_at = Timestamp::from_unix_seconds(u64_of(created_at_seconds)?);
        self.inner
            .finish(created_at)
            .map(Folder)
            .ok_or(SimError::new(Code::Unrecorded))
    }
}

/// The folder `sim run` writes for the netlist JSON, stamped `created_at_seconds` after the unix
/// epoch. A netlist the engine rejects gets a folder whose diagnostics say why.
#[wasm_bindgen]
pub fn run(
    #[wasm_bindgen(unchecked_param_type = "Uint8Array")] netlist: &Netlist,
    #[wasm_bindgen(js_name = createdAtSeconds, unchecked_param_type = "bigint")]
    created_at_seconds: Bigint,
) -> Result<Folder, SimError> {
    let netlist = bytes_of(netlist)?;
    let created_at = Timestamp::from_unix_seconds(u64_of(created_at_seconds)?);
    Ok(Folder(sim_circuit::run(&netlist, created_at).folder))
}
