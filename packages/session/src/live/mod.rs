//! The session loop: one engine session, and the commands that read and drive it.
//!
//! A read at `now` shows the circuit just before any event at `now`. That is the engine's own
//! rule for a probe, whose row at an accepted step is read before any restart there, and the
//! loop holds pin levels to it too: `get` and `expect` leave out an edge at `now` until time
//! moves past it. So an event at t, an edge or a `press`, shows in reads from t + 1 ps. A step
//! from S to E lists every edge in [S, E] once. `--until` first reads its condition as `get`
//! would at S, and a pin's edge at t that meets it ends the step at t + 1 ps.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::path::Path;

use orbe_engine::diagnostic::{Diagnostic as EngineDiagnostic, Excursion};
use orbe_engine::logic::Level as EngineLevel;
use orbe_engine::name::{Name, Quantity, Ref as EngineRef};
use orbe_engine::netlist::{Analysis, Contact, RatingKind};
use orbe_engine::{EdgeCursor, Input, Probe, Refused, Rejected, Session, Setting, Status};
use orbe_scene::{Code as SceneCode, Files, RefMap, View};

use crate::command::{Command, Step};
use crate::condition::Unit;
use crate::duration::Duration;
use crate::reply::{
    Code, Data, Diagnostic, Error, Finding, Got, Level, PartState, Reading, Reply, Snapshot, State,
    Stepped,
};
use crate::target::{QUANTITIES, Ref, Target, names, one_engine, reports_hint, resolve};

mod bound;
mod frame;
mod serial;
mod step;

pub use frame::{Channels, FrameState, Pace, Sample};
pub use step::{CHANGES, Progress};

/// The serial lines a snapshot shows, from the end.
pub const SERIAL_LINES: usize = 10;

use serial::Serial;

/// A named session that holds at most one open scene.
pub struct Live {
    name: String,
    files: Files,
    open: Option<Open>,
    stop: Option<(Duration, String)>,
}

struct Open {
    engine: Session,
    refs: RefMap,
    view: View,
    channels: frame::Bound,
    /// Each switch's contact edges, as `(at_ps, closed)`, after its initial contact: the
    /// scene's events and each input `apply` landed. Bounce is left out.
    contacts: BTreeMap<String, (bool, Vec<(u64, bool)>)>,
    /// Time runs at real speed between commands, since a `play`.
    playing: bool,
    /// What `play`'s ticks ran since a `step` last listed changes, from where that step
    /// ended. The next `step` lists these first.
    unlisted: Option<Stepped>,
    scene: String,
    stop_ps: u64,
    max_step_ps: u64,
    /// The quantities the engine reports for each engine part, by its flat name.
    reports: BTreeMap<String, Vec<Quantity>>,
    /// Each net a chip pin sits on, and the logic signal that reads it, by its index in the
    /// engine's signals.
    signals: BTreeMap<String, usize>,
    /// The pin each logic signal is named by in `step`'s changes.
    signal_pins: Vec<String>,
    ratings: Vec<Rated>,
    /// Past every edge before `now`. A read leaves out an edge at `now` until time moves
    /// past it.
    edges: EdgeCursor,
    /// Past every edge a step has listed, which is every edge through `now` once a step ran.
    listed: EdgeCursor,
    serial: Serial,
}

/// A rated quantity, checked against the probe at `now` as the engine checks accepted steps.
struct Rated {
    kind: RatingKind,
    quantity: EngineRef,
    limit: f64,
}

type Key = (&'static str, Vec<String>);

fn key(d: &EngineDiagnostic) -> Key {
    (d.code(), d.refs().iter().map(ToString::to_string).collect())
}

impl Live {
    /// A session that reads the scenes it opens from `files`.
    pub fn new(name: impl Into<String>, files: Files) -> Live {
        Live {
            name: name.into(),
            files,
            open: None,
            stop: None,
        }
    }

    /// Stops the scenes this session opens at `stop` instead of the 2-hour stop, so a test
    /// reaches it quickly, and `None` puts the 2-hour stop back. A refusal of a stop the
    /// engine would not run names the variable that set it, such as `ORBE_TEST_STOP`.
    pub fn stop_at(&mut self, stop: Option<(Duration, String)>) {
        self.stop = stop;
    }

    pub fn run(&mut self, command: &Command) -> Reply {
        self.run_with(command, &mut |_| ControlFlow::Continue(()))
    }

    /// `run`, with `during` called after each slice of a `step`. A break stops the step there
    /// with `cancelled`. Slicing changes no result, so the session stands where a shorter
    /// step would have left it.
    pub fn run_with(
        &mut self,
        command: &Command,
        during: &mut dyn FnMut(&Progress) -> ControlFlow<()>,
    ) -> Reply {
        let result = match (command, &mut self.open) {
            (Command::Open(scene), _) => self.open(scene),
            (Command::Close, Some(open)) => {
                let at_ps = open.engine.now().as_ps();
                self.open = None;
                Ok(Data::Closed { at_ps })
            }
            (_, None) => Err(failed(
                Error::new(Code::NoSession, "no session is open", "run `open` first"),
                Data::None,
            )),
            (Command::Press(r), Some(open)) => open.contact(r, true),
            (Command::Release(r), Some(open)) => open.contact(r, false),
            (Command::Get(r), Some(open)) => open.get(r).map(Data::Got).map_err(fail),
            (Command::Step(step), Some(open)) => open.step(step, during),
            (Command::Play, Some(open)) => open.play(),
            (Command::Pause, Some(open)) => {
                open.playing = false;
                Ok(Data::Paced { playing: false })
            }
            (Command::Snapshot, Some(open)) => Ok(open.snapshot()),
            (Command::Expect(condition), Some(open)) => open.expect(condition),
        };
        match result {
            Ok(data) => self.reply(data, None),
            Err(failed) => self.reply(failed.data, Some(failed.error)),
        }
    }

    /// A reply with no command behind it, such as a test line that does not parse.
    pub fn refuse(&self, error: Error) -> Reply {
        self.reply(Data::None, Some(error))
    }

    /// A reply that carries what the session's process did for its client, such as `show`'s
    /// address, with the session's time and state.
    pub fn report(&self, data: Data) -> Reply {
        self.reply(data, None)
    }

    /// Whether time runs between commands, since a `play`.
    pub fn playing(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.playing)
    }

    /// One `play` tick, `wall` after the one before: advances a playing session by as much
    /// simulated time, up to one [`TICK`](crate::viewer::TICK) at speed 1, or to its stop when
    /// that is nearer, as a `step` would, and keeps its changes for the next `step` to list. Play
    /// ends at the stop or a fault. Returns the tick's pace while it still plays.
    pub fn tick(&mut self, wall: std::time::Duration) -> Option<Pace> {
        let open = self.open.as_mut().filter(|open| open.playing)?;
        let simulated = wall.min(crate::viewer::TICK);
        let ps = simulated.as_nanos() as u64 * 1000;
        let length = ps.min(open.stop_ps - open.engine.now().as_ps());
        let step = Step::For(Duration::from_ps(length));
        let ran = open.step(&step, &mut |_| ControlFlow::Continue(()));
        open.playing = ran.is_ok() && open.engine.status() == Status::Running;
        open.unlisted = match ran.map_err(|failed| failed.data) {
            Ok(Data::Stepped(stepped)) | Err(Data::Stepped(stepped)) => Some(stepped),
            _ => None,
        };
        open.playing.then(|| Pace {
            speed: 1.0,
            ratio: simulated.as_secs_f64() / wall.as_secs_f64().max(f64::EPSILON),
        })
    }

    fn reply(&self, data: Data, error: Option<Error>) -> Reply {
        let (t_ps, state) = match &self.open {
            None => (0, State::Closed),
            Some(open) => (open.engine.now().as_ps(), open.state()),
        };
        Reply {
            session: self.name.clone(),
            t_ps,
            state,
            data,
            error,
        }
    }

    /// Checks the scene as `orbe check` does and opens it. A refused scene leaves the session
    /// that was open as it was.
    fn open(&mut self, scene: &Path) -> Outcome {
        let shown = scene.display().to_string();
        if !self.files.is_file(scene) {
            return Err(fail(Error::new(
                Code::IoError,
                format!("no scene file at {shown}"),
                "write the scene, or name its path",
            )));
        }
        let dir = scene.parent().unwrap_or(Path::new(""));
        let mut netlist =
            orbe_scene::read(&self.files, scene).map_err(|found| refused(dir, &shown, found))?;
        if let Some((stop, variable)) = &self.stop {
            let stop = shorter_stop(&netlist.json, *stop, variable).map_err(fail)?;
            let at = |ps: u64| format!("\"stop_ps\": {ps}");
            netlist.json = netlist
                .json
                .replacen(&at(orbe_scene::STOP_PS), &at(stop), 1);
        }
        let open = Open::new(&netlist.json, netlist.refs, netlist.view, shown.clone())?;
        let data = Data::Opened {
            scene: shown,
            parts: open.refs.top.clone(),
            engine_parts: netlist.parts,
            nets: netlist.nets.len(),
            diagnostics: open.diagnostics(&open.engine.probe()),
        };
        self.open = Some(open);
        Ok(data)
    }
}

/// The engine runs no stop shorter than the scene's max step or trace grid.
fn shorter_stop(json: &str, stop: Duration, variable: &str) -> Result<u64, Error> {
    let netlist =
        orbe_engine::netlist::parse(json.as_bytes()).expect("the reader writes a netlist");
    let Analysis::Transient(transient) = netlist.analysis() else {
        unreachable!("a session runs a transient");
    };
    let least = transient.max_step.max(transient.grid);
    let refuse = |message: String, hint: String| Err(Error::new(Code::Usage, message, hint));
    if stop.as_ps() > orbe_scene::STOP_PS {
        return refuse(
            format!("{variable}={stop} is longer than the 2-hour stop"),
            format!(
                "set {variable} to at most {}, or unset it",
                t(orbe_scene::STOP_PS)
            ),
        );
    }
    if stop.as_ps() < least {
        return refuse(
            format!(
                "{variable}={stop} is shorter than {}, the shortest stop the engine runs for this scene",
                t(least)
            ),
            format!("set {variable} to at least {}, or unset it", t(least)),
        );
    }
    Ok(stop.as_ps())
}

/// `orbe check`'s findings as one refusal: `part_missing` or `firmware_missing` when every
/// finding is that, else `scene_invalid`. One finding keeps its own message and hint.
fn refused(dir: &Path, scene: &str, found: Vec<orbe_scene::Diagnostic>) -> Box<Failed> {
    let placed: Vec<Finding> = found
        .into_iter()
        .map(|d| {
            Finding::new(orbe_scene::Diagnostic {
                file: dir.join(&d.file).display().to_string(),
                ..d
            })
        })
        .collect();
    let all = |code: SceneCode| placed.iter().all(|d| d.code == code.as_str());
    let code = if placed.is_empty() {
        Code::SceneInvalid
    } else if all(SceneCode::PartMissing) {
        Code::PartMissing
    } else if all(SceneCode::FirmwareMissing) {
        Code::FirmwareMissing
    } else {
        Code::SceneInvalid
    };
    let error = match placed.as_slice() {
        [one] => Error::new(code, one.message.clone(), one.hint.clone()),
        many => Error::new(
            code,
            format!("{} errors in {scene}", many.len()),
            "fix each finding, then run `open` again",
        ),
    };
    failed(error, Data::Invalid(placed))
}

fn engine_name(name: &str) -> Name {
    Name::new(name).expect("the reader writes valid engine names")
}

fn quantity(part: &str, quantity: Quantity) -> EngineRef {
    EngineRef::Quantity(engine_name(part), quantity)
}

fn unit_of(quantity: Quantity) -> Unit {
    match quantity {
        Quantity::V => Unit::Volt,
        Quantity::P => Unit::Watt,
        _ => Unit::Ampere,
    }
}

fn level(level: EngineLevel) -> Level {
    match level {
        EngineLevel::High => Level::High,
        EngineLevel::Low => Level::Low,
    }
}

fn t(ps: u64) -> Duration {
    Duration::from_ps(ps)
}

fn rejected(rejected: Rejected, target: &str) -> Error {
    let (code, message, hint) = match rejected {
        Rejected::Ended => (
            Code::Ended,
            "the session has ended".to_string(),
            "run `open` to start again",
        ),
        Rejected::UnknownPart => (
            Code::UnknownRef,
            format!("the engine has no part `{target}`"),
            "",
        ),
        Rejected::NotSettable => (
            Code::NotSettable,
            format!("`{target}` takes no input"),
            "press a switch",
        ),
        Rejected::WrongSetting => (
            Code::WrongSetting,
            format!("`{target}` is not a switch"),
            "press a switch",
        ),
        Rejected::Value(_) => (
            Code::Value,
            format!("the engine refused the value for `{target}`"),
            "",
        ),
        Rejected::PastStop => (
            Code::PastStop,
            "the input would land past the session's stop".to_string(),
            "run `open` to start again",
        ),
        Rejected::NoChange => (
            Code::NoChange,
            format!("`{target}` is already in that state"),
            "",
        ),
    };
    Error::new(code, message, hint)
}

/// A command that failed, with what it did before it failed, such as a `step`'s changes.
struct Failed {
    error: Error,
    data: Data,
}

type Outcome = Result<Data, Box<Failed>>;

fn failed(error: Error, data: Data) -> Box<Failed> {
    Box::new(Failed { error, data })
}

fn fail(error: Error) -> Box<Failed> {
    failed(error, Data::None)
}

impl Open {
    fn new(json: &str, refs: RefMap, view: View, scene: String) -> Result<Open, Box<Failed>> {
        let engine = Session::open_document(json.as_bytes()).map_err(|refused| {
            let (code, message) = match &refused {
                Refused::Op => (
                    "netlist.analysis",
                    "the netlist has no time to advance".to_string(),
                ),
                Refused::Failed(d) => (
                    d.code(),
                    format!("the engine could not start: {}", d.code()),
                ),
                Refused::Invalid(found) => (
                    found.first().map_or("netlist.syntax", |d| d.code()),
                    "the engine refused the netlist the reader wrote".to_string(),
                ),
            };
            fail(Error::new(
                Code::Engine(code.to_string()),
                message,
                "report this with the scene",
            ))
        })?;
        let netlist = orbe_engine::netlist::parse(json.as_bytes())
            .expect("the engine opened the same netlist");
        let Analysis::Transient(transient) = netlist.analysis() else {
            unreachable!("a session runs a transient");
        };
        let ratings = netlist
            .parts()
            .flat_map(|(_, part)| {
                part.model.ratings().into_iter().map(|(kind, value)| Rated {
                    kind,
                    quantity: EngineRef::Quantity(part.name.clone(), part.model.rated(kind)),
                    limit: value.si.get(),
                })
            })
            .collect();
        let mut reports: BTreeMap<String, Vec<Quantity>> = BTreeMap::new();
        for column in engine.probe().refs() {
            if let EngineRef::Quantity(part, q) = column {
                reports
                    .entry(part.as_str().to_string())
                    .or_default()
                    .push(*q);
            }
        }
        let (signals, signal_pins) = signals(&engine, &refs);
        let edges = engine.edge_cursor();
        let contacts = frame::contacts(json);
        let mut open = Open {
            engine,
            refs,
            view,
            channels: frame::Bound::default(),
            contacts,
            playing: false,
            unlisted: None,
            scene,
            stop_ps: transient.stop.as_ps(),
            max_step_ps: transient.max_step,
            reports,
            signals,
            signal_pins,
            ratings,
            edges,
            listed: edges,
            serial: Serial::default(),
        };
        open.channels = open.bind_channels();
        Ok(open)
    }

    fn state(&self) -> State {
        match self.engine.status() {
            Status::Running if self.playing => State::Playing,
            Status::Running => State::Paused,
            Status::Stopped => State::Stopped,
            Status::Faulted => State::Faulted,
        }
    }

    /// Whether a part is a board with holes, such as a breadboard. A leg in a hole joins the
    /// hole's net, so the board would otherwise join every net a part plugged into it has.
    fn holed(&self, top: &str) -> bool {
        self.view
            .parts
            .get(&self.refs.parts[top].ty)
            .is_some_and(|data| !data.strips.is_empty())
    }

    /// The nets that join two or more of the scene's own parts, a board with holes aside.
    fn wired_nets(&self) -> BTreeSet<&str> {
        let mut owners: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for top in self.refs.top.iter().filter(|top| !self.holed(top)) {
            for net in self.refs.parts[top].pins.values().flatten() {
                owners.entry(net).or_default().insert(top);
            }
        }
        owners
            .into_iter()
            .filter(|(_, tops)| tops.len() > 1)
            .map(|(net, _)| net)
            .collect()
    }

    fn play(&mut self) -> Outcome {
        match self.engine.status() {
            Status::Faulted => return Err(fail(self.faulted())),
            Status::Stopped => {
                return Err(fail(Error::new(
                    Code::Ended,
                    format!("the session reached its {} stop", self.stop_name()),
                    "run `open` to start again",
                )));
            }
            Status::Running => {}
        }
        self.playing = true;
        Ok(Data::Paced { playing: true })
    }

    fn resolve(&self, r: &Ref) -> Result<Target, Error> {
        resolve(&self.refs, r, &|engine| {
            self.reports.get(engine).cloned().unwrap_or_default()
        })
    }

    /// The engine's diagnostics, the fault, and each rated quantity the probe has over its
    /// rating that the engine has not yet found on an accepted step.
    fn findings(&self, probe: &Probe) -> Vec<EngineDiagnostic> {
        let mut found = self.engine.diagnostics();
        let known: BTreeSet<Key> = found.iter().map(key).collect();
        let at = probe.at();
        for rated in &self.ratings {
            let Some(magnitude) = probe.get(&rated.quantity).map(|v| v.get().abs()) else {
                continue;
            };
            if magnitude <= rated.limit {
                continue;
            }
            let excursion = Excursion {
                quantity: rated.quantity.clone(),
                limit: rated.limit,
                peak: magnitude,
                peak_at: at,
                first: at,
                over_ps: None,
            };
            let over = match rated.kind {
                RatingKind::Current => EngineDiagnostic::RatingCurrent(excursion),
                RatingKind::Power => EngineDiagnostic::RatingPower(excursion),
            };
            if !known.contains(&key(&over)) {
                found.push(over);
            }
        }
        found.extend(self.engine.fault().cloned());
        orbe_engine::run_folder::sort_diagnostics(&mut found);
        found
    }

    fn diagnostics(&self, probe: &Probe) -> Vec<Diagnostic> {
        self.findings(probe).iter().map(Diagnostic::new).collect()
    }

    fn contact(&mut self, r: &Ref, pressed: bool) -> Outcome {
        let path = match self.resolve(r).map_err(fail)? {
            Target::Part(path) => path,
            _ => {
                return Err(fail(Error::new(
                    Code::NotSettable,
                    format!("{r} is not a part"),
                    "press a switch by its part name, such as `@btn1`",
                )));
            }
        };
        let engine = one_engine(&path, &self.refs.parts[&path], Code::NotSettable).map_err(fail)?;
        let contact = if pressed {
            Contact::Closed
        } else {
            Contact::Open
        };
        let input = Input {
            part: engine_name(&engine),
            at: self.engine.now(),
            setting: Setting::Contact(contact),
        };
        let applied = self
            .engine
            .apply(&input)
            .map_err(|e| fail(rejected(e, &path)))?;
        if let Some((_, edges)) = self.contacts.get_mut(&engine) {
            edges.push((applied.at.as_ps(), pressed));
        }
        let data = Data::Landed {
            target: path,
            pressed,
            landed_ps: applied.at.as_ps(),
        };
        match applied.status {
            Status::Faulted => Err(failed(self.faulted(), data)),
            _ => Ok(data),
        }
    }

    fn faulted(&self) -> Error {
        let code = self.engine.fault().map_or("unknown", |d| d.code());
        Error::new(
            Code::Faulted,
            format!("the simulation faulted with {code}"),
            "run `snapshot` to see the diagnostics, fix the scene or firmware, then `open` again",
        )
    }

    fn get(&self, r: &Ref) -> Result<Got, Error> {
        let (target, value) = match self.resolve(r)? {
            Target::Pin { path, pin, net } => {
                let value = self.pin(&path, &pin, net.as_deref())?;
                (format!("{path}.{pin}"), value)
            }
            Target::Quantity {
                path,
                engine,
                quantity: q,
            } => (
                format!("{path}.{}", q.as_str()),
                Reading::Quantity {
                    unit: unit_of(q),
                    value: self.read(&quantity(&engine, q)),
                },
            ),
            Target::Part(path) => {
                let value = self.part(&path).ok_or_else(|| {
                    Error::new(
                        Code::NoReading,
                        format!("`{path}` has no reading of its own"),
                        match self.refs.parts[&path].pins.keys().next() {
                            Some(hole) if self.holed(&path) => {
                                format!("read one of its holes, such as `{path}.{hole}`")
                            }
                            _ => format!("read one of its {}", names(&self.refs.parts[&path], &[])),
                        },
                    )
                })?;
                (path, value)
            }
        };
        Ok(Got { target, value })
    }

    /// A column the engine probes, or why the ref has none, such as `i` on a chip.
    fn column(&self, path: &str, engine: &str, q: Quantity) -> Result<EngineRef, Error> {
        let reported = self.reports.get(engine).map_or(&[][..], Vec::as_slice);
        if reported.contains(&q) {
            return Ok(quantity(engine, q));
        }
        Err(Error::new(
            Code::NoReading,
            format!("the engine reports no `{path}.{}`", q.as_str()),
            reports_hint(path, reported),
        ))
    }

    /// A column `column` checked, or a net, which is always one.
    fn read(&self, column: &EngineRef) -> f64 {
        self.engine
            .probe()
            .get(column)
            .expect("a checked column")
            .get()
    }

    fn level_before_now(&self, signal: usize) -> Level {
        let now = self.engine.now();
        let at_now = self.engine.edges_since(self.edges).and_then(|(edges, _)| {
            edges
                .iter()
                .find(|e| e.signal == signal && e.at == now)
                .copied()
        });
        match at_now {
            Some(edge) if edge.level == EngineLevel::High => Level::Low,
            Some(_) => Level::High,
            None => level(self.engine.levels()[signal]),
        }
    }

    fn pin(&self, path: &str, pin: &str, net: Option<&str>) -> Result<Reading, Error> {
        let net = net.ok_or_else(|| self.not_simulated(path, pin))?;
        Ok(Reading::Pin {
            level: self.signals.get(net).map(|&k| self.level_before_now(k)),
            v: self.read(&EngineRef::Net(engine_name(net))),
        })
    }

    /// Why a pin on no engine net has no reading, with a pin of the same part that has a
    /// level, else a voltage.
    fn not_simulated(&self, path: &str, pin: &str) -> Error {
        let pins = &self.refs.parts[path].pins;
        let leveled = pins
            .iter()
            .find(|(_, net)| net.as_ref().is_some_and(|n| self.signals.contains_key(n)));
        let hint = match leveled.or_else(|| pins.iter().find(|(_, net)| net.is_some())) {
            Some((other, _)) => {
                format!("read a pin the engine simulates, such as `@{path}.{other}`")
            }
            None => format!("`{path}` has no pin the engine simulates"),
        };
        Error::new(
            Code::NoReading,
            format!("`{path}.{pin}` is not simulated"),
            hint,
        )
    }

    /// A part's `on` reading and its engine quantities. `None` when it has neither.
    fn part(&self, path: &str) -> Option<Reading> {
        let engine = one_engine(path, &self.refs.parts[path], Code::NoReading).ok()?;
        let probe = self.engine.probe();
        let quantities: Vec<(String, Unit, f64)> = self
            .reports
            .get(&engine)?
            .iter()
            .map(|&q| {
                let value = probe.get(&quantity(&engine, q)).expect("a reported column");
                (q.as_str().to_string(), unit_of(q), value.get())
            })
            .collect();
        let on = self
            .on_threshold(path)
            .ok()
            .map(|(i, above)| probe.get(&i).expect("a checked column").get() > above);
        (on.is_some() || !quantities.is_empty()).then_some(Reading::Part { on, quantities })
    }

    /// The column and threshold of a part's `on` reading.
    fn on_threshold(&self, path: &str) -> Result<(EngineRef, f64), Error> {
        let part = &self.refs.parts[path];
        let missing = || {
            Error::new(
                Code::NoReading,
                format!("`{path}` states no `on` reading"),
                format!(
                    "{} has no readings.on with `above` in its part file",
                    part.ty
                ),
            )
        };
        let reading = part.readings.get("on").ok_or_else(missing)?;
        let above = reading.above.as_ref().ok_or_else(missing)?;
        let q = QUANTITIES
            .into_iter()
            .find(|q| q.as_str() == reading.of)
            .ok_or_else(missing)?;
        if above.unit != unit_of(q).as_str() {
            return Err(Error::new(
                Code::NoReading,
                format!(
                    "`{path}`'s `on` reading is in {}, but `{}` is in {}",
                    above.unit,
                    reading.of,
                    unit_of(q).as_str()
                ),
                format!("fix readings.on in {}", part.ty),
            ));
        }
        let engine = one_engine(path, part, Code::NoReading)?;
        Ok((self.column(path, &engine, q)?, above.value))
    }

    fn snapshot(&self) -> Data {
        let wired = self.wired_nets();
        let mut parts = Vec::new();
        for top in &self.refs.top {
            let part = &self.refs.parts[top];
            let pins = part
                .pins
                .iter()
                .filter(|_| part.engine.is_empty() && !self.holed(top))
                .filter_map(|(pin, net)| {
                    let net = net.as_deref().filter(|n| wired.contains(n))?;
                    Some((pin.clone(), self.pin(top, pin, Some(net)).ok()?))
                })
                .collect();
            parts.push(PartState {
                name: top.clone(),
                ty: part.ty.clone(),
                value: self.part(top),
                pins,
            });
        }
        let lines: Vec<&str> = self.serial.text().lines().collect();
        let serial = lines[lines.len().saturating_sub(SERIAL_LINES)..].join("\n");
        Data::Snapshot(Snapshot {
            scene: self.scene.clone(),
            parts,
            serial,
            serial_held: self.serial.held(),
            diagnostics: self.diagnostics(&self.engine.probe()),
        })
    }
}

/// Each chip logic signal by the net its pin sits on, and the pin each is named by: the one
/// pin of the chip's top-level part on that net, such as `nano.D13`, else the chip's own pin,
/// such as `nano.mcu.PB5`. The scene's order and the other parts on the net play no part.
fn signals(engine: &Session, refs: &RefMap) -> (BTreeMap<String, usize>, Vec<String>) {
    let owners: BTreeMap<&str, (&str, &BTreeMap<String, String>)> = refs
        .parts
        .iter()
        .flat_map(|(path, p)| {
            p.engine
                .iter()
                .map(move |e| (e.name.as_str(), (path.as_str(), &e.pins)))
        })
        .collect();
    let mut by_net = BTreeMap::new();
    let mut names = Vec::new();
    for (k, signal) in engine.signals().iter().enumerate() {
        let found = match signal {
            EngineRef::Logic(part, pin) => owners.get(part.as_str()).and_then(|(path, pins)| {
                Some((*path, pin.as_str(), pins.get(pin.as_str())?.as_str()))
            }),
            _ => None,
        };
        let Some((path, pin, net)) = found else {
            names.push(signal.to_string());
            continue;
        };
        by_net.entry(net.to_string()).or_insert(k);
        let top = path.split('.').next().unwrap_or(path);
        let on_net: Vec<&String> = refs.parts[top]
            .pins
            .iter()
            .filter(|(_, n)| n.as_deref() == Some(net))
            .map(|(p, _)| p)
            .collect();
        names.push(match on_net.as_slice() {
            [one] => format!("{top}.{one}"),
            _ => format!("{path}.{pin}"),
        });
    }
    (by_net, names)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn stopping_at_50ms() -> Live {
        let scene = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../blocks/nano-button-led/scene.jsonc");
        let netlist = orbe_scene::read(&Files::disk(), &scene).unwrap();
        let stop = format!("\"stop_ps\": {}", orbe_scene::STOP_PS);
        assert!(netlist.json.contains(&stop), "{}", netlist.json);
        let json = netlist.json.replace(&stop, "\"stop_ps\": 50000000000");
        let mut live = Live::new("t", Files::disk());
        live.open = Some(
            Open::new(&json, netlist.refs, netlist.view, "scene.jsonc".into())
                .unwrap_or_else(|f| panic!("{:?}", f.error)),
        );
        live
    }

    fn step(live: &mut Live, length: &str) -> Reply {
        let command = Command::parse(&format!("step {length}"), Path::new(""))
            .unwrap()
            .unwrap();
        live.run(&command)
    }

    #[test]
    fn a_step_that_lands_on_the_stop_succeeds_and_the_next_has_ended() {
        let mut live = stopping_at_50ms();
        let refused = step(&mut live, "60ms");
        assert_eq!(
            refused.error.unwrap().hint,
            "the session has 50ms left; step at most that"
        );
        assert_eq!(refused.t_ps, 0, "a refused step takes no time");

        let reply = step(&mut live, "50ms");
        assert!(reply.ok(), "{}", reply.text());
        assert_eq!((reply.t_ps, reply.state), (50_000_000_000, State::Stopped));
        let after = step(&mut live, "1ms");
        assert_eq!(after.error.unwrap().code, Code::Ended);
    }

    #[test]
    fn an_until_near_the_stop_meets_at_once_or_ends_at_the_stop() {
        let mut live = stopping_at_50ms();
        assert!(step(&mut live, "45ms").ok());
        let held = step(&mut live, "--until @led1 off");
        assert!(held.ok(), "{}", held.text());
        assert_eq!(
            held.t_ps, 45_000_000_000,
            "a condition that holds takes no time"
        );

        let reached = step(&mut live, "--until @led1 on --timeout 10ms");
        let error = reached.error.as_ref().unwrap();
        assert_eq!(error.code, Code::Ended, "{}", reached.text());
        assert_eq!(
            error.message,
            "the session reached its stop before the condition held"
        );
        assert_eq!(
            (reached.t_ps, reached.state),
            (50_000_000_000, State::Stopped)
        );
    }

    #[test]
    fn an_until_at_the_stop_reads_its_condition_there() {
        let mut live = stopping_at_50ms();
        assert!(step(&mut live, "50ms").ok());
        let held = step(&mut live, "--until @led1 off --timeout 0s");
        assert!(held.ok(), "{}", held.text());
        assert_eq!(held.text(), "t=50ms (from 50ms), @led1 off at 50ms");

        let unmet = step(&mut live, "--until @led1 on");
        assert_eq!(unmet.error.as_ref().unwrap().code, Code::Ended);
        let actual = &unmet.check().unwrap().actual;
        assert!(actual.starts_with("led1 off, i="), "{}", unmet.text());
    }
}
