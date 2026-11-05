//! The session loop: one engine session, and the commands that read and drive it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orbe_scene::{Code as SceneCode, RefMap};
use sim_circuit::{EdgeCursor, Input, Refused, Rejected, SerialCursor, Session, Setting, Status};
use sim_core::logic::Level as EngineLevel;
use sim_core::name::{Name, Quantity, Ref as EngineRef};
use sim_core::netlist::Contact;
use sim_core::time::Time;

use crate::command::{Command, Step};
use crate::condition::{Condition, Op, State as Want, Unit};
use crate::duration::Duration;
use crate::reply::{
    Change, Check, Code, Data, Diagnostic, Error, Got, Level, PartState, Reading, Reply, Snapshot,
    State, Stepped, number,
};
use crate::target::{Ref, Target, one_engine, resolve};

/// `step` advances in slices this long, so `--until` sees each one.
const SLICE: Duration = Duration::from_ms(1);
const CHANGES: usize = 20;
/// The serial a session keeps for `serial contains` and `snapshot`, in bytes.
const SERIAL_KEPT: usize = 64 * 1024;

/// A named session that holds at most one open scene.
pub struct Live {
    name: String,
    open: Option<Open>,
}

struct Open {
    engine: Session,
    refs: RefMap,
    scene: String,
    /// Each chip logic signal, by the net its pin sits on.
    signals: BTreeMap<String, EngineRef>,
    /// The pin each logic signal reads, named from the scene, for `step`'s changes.
    signal_pins: Vec<String>,
    edges: EdgeCursor,
    serial_cursor: SerialCursor,
    serial: String,
}

/// What a condition reads, resolved against the open scene.
enum Bound {
    On {
        part: String,
        i: EngineRef,
        above: f64,
        want: bool,
    },
    Level {
        pin: String,
        signal: EngineRef,
        want: Level,
    },
    Compare {
        label: String,
        column: EngineRef,
        op: Op,
        value: f64,
        unit: Unit,
    },
    SerialContains(String),
    NoDiagnostics,
}

impl Live {
    pub fn new(name: impl Into<String>) -> Live {
        Live {
            name: name.into(),
            open: None,
        }
    }

    pub fn run(&mut self, command: &Command) -> Reply {
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
            (Command::Get(r), Some(open)) => open.get(r),
            (Command::Step(step), Some(open)) => open.step(step),
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

    fn reply(&self, data: Data, error: Option<Error>) -> Reply {
        let (t_ps, state) = match &self.open {
            None => (0, State::Closed),
            Some(open) => (
                open.engine.now().as_ps(),
                match open.engine.status() {
                    Status::Running => State::Paused,
                    Status::Stopped => State::Stopped,
                    Status::Faulted => State::Faulted,
                },
            ),
        };
        Reply {
            session: self.name.clone(),
            t_ps,
            state,
            data,
            error,
        }
    }

    /// Checks the scene as `orbe check` does and opens it. A refused scene leaves no session.
    fn open(&mut self, scene: &Path) -> Outcome {
        self.open = None;
        let shown = scene.display().to_string();
        if !scene.is_file() {
            return Err(failed(
                Error::new(
                    Code::IoError,
                    format!("no scene file at {shown}"),
                    "write the scene, or name its path",
                ),
                Data::None,
            ));
        }
        let dir = scene.parent().unwrap_or(Path::new(""));
        let netlist = orbe_scene::read(scene).map_err(|found| refused(dir, &shown, found))?;
        let engine = Session::open_document(netlist.json.as_bytes()).map_err(|refused| {
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
                Code::Engine(code),
                message,
                "report this with the scene",
            ))
        })?;
        let open = Open::new(engine, netlist.refs, shown.clone());
        let data = Data::Opened {
            scene: shown,
            parts: open.refs.top.clone(),
            engine_parts: netlist.parts,
            nets: netlist.nets.len(),
            diagnostics: open.diagnostics(),
        };
        self.open = Some(open);
        Ok(data)
    }
}

/// `orbe check`'s findings as one refusal: `part_missing` or `firmware_missing` when every
/// finding is that, else `scene_invalid`. One finding keeps its own message and hint.
fn refused(dir: &Path, scene: &str, found: Vec<orbe_scene::Diagnostic>) -> Box<Failed> {
    let placed: Vec<orbe_scene::Diagnostic> = found
        .into_iter()
        .map(|d| orbe_scene::Diagnostic {
            file: dir.join(&d.file).display().to_string(),
            ..d
        })
        .collect();
    let code = match placed.first().map(|d| d.code) {
        Some(first @ (SceneCode::PartMissing | SceneCode::FirmwareMissing))
            if placed.iter().all(|d| d.code == first) =>
        {
            if first == SceneCode::PartMissing {
                Code::PartMissing
            } else {
                Code::FirmwareMissing
            }
        }
        _ => Code::SceneInvalid,
    };
    let error = match placed.as_slice() {
        [one] => Error::new(code, one.message.clone(), one.hint.clone()),
        many => Error::new(
            code,
            format!("{} errors in {scene}", many.len()),
            "fix each finding above, then run `open` again",
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
    fn new(engine: Session, refs: RefMap, scene: String) -> Open {
        let mut pins_by_net: BTreeMap<&str, String> = BTreeMap::new();
        for top in &refs.top {
            for (pin, net) in &refs.parts[top].pins {
                if let Some(net) = net {
                    pins_by_net
                        .entry(net)
                        .or_insert_with(|| format!("{top}.{pin}"));
                }
            }
        }
        let engine_pins: BTreeMap<&str, &BTreeMap<String, String>> = refs
            .parts
            .values()
            .flat_map(|p| &p.engine)
            .map(|e| (e.name.as_str(), &e.pins))
            .collect();
        let mut signals = BTreeMap::new();
        let mut signal_pins = Vec::new();
        for signal in engine.signals() {
            let net = match signal {
                EngineRef::Logic(part, pin) => engine_pins
                    .get(part.as_str())
                    .and_then(|pins| pins.get(pin.as_str())),
                _ => None,
            };
            if let Some(net) = net {
                signals.entry(net.clone()).or_insert_with(|| signal.clone());
            }
            let shown = net.and_then(|n| pins_by_net.get(n.as_str()).cloned());
            signal_pins.push(shown.unwrap_or_else(|| signal.to_string()));
        }
        let edges = engine.edge_cursor();
        Open {
            engine,
            refs,
            scene,
            signals,
            signal_pins,
            edges,
            serial_cursor: SerialCursor::default(),
            serial: String::new(),
        }
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut found: Vec<Diagnostic> = self
            .engine
            .diagnostics()
            .iter()
            .map(Diagnostic::new)
            .collect();
        found.extend(self.engine.fault().map(Diagnostic::new));
        found
    }

    fn contact(&mut self, r: &Ref, pressed: bool) -> Outcome {
        let path = match resolve(&self.refs, r).map_err(fail)? {
            Target::Part(path) => path,
            _ => {
                return Err(fail(Error::new(
                    Code::NotSettable,
                    format!("{r} is not a part"),
                    "press a switch by its part name, such as `@btn1`",
                )));
            }
        };
        let engine = one_engine(&path, &self.refs.parts[&path]).map_err(fail)?;
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

    fn get(&self, r: &Ref) -> Outcome {
        let target = resolve(&self.refs, r).map_err(fail)?;
        let (name, value) = match target {
            Target::Pin { path, pin, net } => (
                format!("{path}.{pin}"),
                self.pin(&format!("{path}.{pin}"), net.as_deref())
                    .map_err(fail)?,
            ),
            Target::Quantity {
                path,
                engine,
                quantity: q,
            } => {
                let name = format!("{path}.{}", q.as_str());
                let column = self.column(&name, quantity(&engine, q)).map_err(fail)?;
                let value = Reading::Quantity {
                    unit: unit_of(q).as_str(),
                    value: self.read(&column),
                };
                (name, value)
            }
            Target::Part(path) => {
                let value = self.part(&path).ok_or_else(|| {
                    fail(Error::new(
                        Code::NoReading,
                        format!("`{path}` has no engine part of its own to read"),
                        "get one of its pins, such as `@nano.D13`",
                    ))
                })?;
                (path, value)
            }
        };
        Ok(Data::Got(Got {
            target: name,
            value,
        }))
    }

    /// A column the engine probes, or why the ref has none, such as `i` on a chip.
    fn column(&self, name: &str, column: EngineRef) -> Result<EngineRef, Error> {
        if self.engine.probe().get(&column).is_some() {
            return Ok(column);
        }
        Err(Error::new(
            Code::NoReading,
            format!("the engine reports no `{name}`"),
            "a chip reports no i, v or p; read one of its pins",
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

    fn pin(&self, name: &str, net: Option<&str>) -> Result<Reading, Error> {
        let net = net.ok_or_else(|| {
            Error::new(
                Code::NoReading,
                format!("`{name}` is not simulated"),
                "read a pin the engine simulates",
            )
        })?;
        let level = self
            .signals
            .get(net)
            .and_then(|s| self.engine.level(s))
            .map(|l| match l {
                EngineLevel::High => Level::High,
                EngineLevel::Low => Level::Low,
            });
        Ok(Reading::Pin {
            level,
            v: self.read(&EngineRef::Net(engine_name(net))),
        })
    }

    /// A part's `on` reading and its engine quantities. `None` for a part with no engine part.
    fn part(&self, path: &str) -> Option<Reading> {
        let part = &self.refs.parts[path];
        let engine = one_engine(path, part).ok()?;
        let quantities = [Quantity::I, Quantity::V, Quantity::P]
            .into_iter()
            .filter_map(|q| {
                let value = self.engine.probe().get(&quantity(&engine, q))?.get();
                Some((q.as_str(), unit_of(q).as_str(), value))
            })
            .collect();
        Some(Reading::Part {
            on: self
                .on_threshold(path)
                .ok()
                .map(|(i, above)| self.read(&i) > above),
            quantities,
        })
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
        let q = [Quantity::I, Quantity::V, Quantity::P]
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
        let engine = one_engine(path, part)?;
        let column = self.column(&format!("{path}.{}", reading.of), quantity(&engine, q))?;
        Ok((column, above.value))
    }

    fn bind(&self, condition: &Condition) -> Result<Bound, Error> {
        Ok(match condition {
            Condition::NoDiagnostics => Bound::NoDiagnostics,
            Condition::SerialContains(text) => Bound::SerialContains(text.clone()),
            Condition::State(r, want) => match (resolve(&self.refs, r)?, want) {
                (Target::Part(path), Want::On | Want::Off) => {
                    let (i, above) = self.on_threshold(&path)?;
                    Bound::On {
                        part: path,
                        i,
                        above,
                        want: *want == Want::On,
                    }
                }
                (Target::Pin { path, pin, net }, Want::High | Want::Low) => {
                    let name = format!("{path}.{pin}");
                    let signal = net.as_ref().and_then(|n| self.signals.get(n)).cloned().ok_or_else(|| {
                        Error::new(
                            Code::NoReading,
                            format!("`{name}` has no logic level, because no chip pin shares its net"),
                            format!("compare its voltage instead, such as `{r} > 2.5V`"),
                        )
                    })?;
                    let want = if *want == Want::High {
                        Level::High
                    } else {
                        Level::Low
                    };
                    Bound::Level {
                        pin: name,
                        signal,
                        want,
                    }
                }
                (_, want) => {
                    return Err(Error::new(
                        Code::NoReading,
                        format!("{r} cannot be {}", want.as_str()),
                        "a part with an `on` reading is on or off, and a pin on a chip's net is high or low",
                    ));
                }
            },
            Condition::Compare(r, op, amount) => {
                let (label, column, unit) = match resolve(&self.refs, r)? {
                    Target::Pin {
                        path,
                        pin,
                        net: Some(net),
                    } => (
                        format!("{path}.{pin}"),
                        EngineRef::Net(engine_name(&net)),
                        Unit::Volt,
                    ),
                    Target::Quantity {
                        path,
                        engine,
                        quantity: q,
                    } => (
                        format!("{path}.{}", q.as_str()),
                        quantity(&engine, q),
                        unit_of(q),
                    ),
                    _ => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} has no one value to compare"),
                            "compare a pin's voltage or a quantity, such as `@led1.i > 5mA`",
                        ));
                    }
                };
                if unit != amount.unit {
                    return Err(Error::new(
                        Code::NoReading,
                        format!("{r} is in {}, not {}", unit.as_str(), amount.unit.as_str()),
                        format!("write the value in {}", unit.as_str()),
                    ));
                }
                let column = self.column(&label, column)?;
                Bound::Compare {
                    label,
                    column,
                    op: *op,
                    value: amount.value,
                    unit,
                }
            }
        })
    }

    /// Whether the condition holds now, and what was seen.
    fn observe(&self, bound: &Bound) -> (bool, String) {
        match bound {
            Bound::On {
                part,
                i,
                above,
                want,
            } => {
                let value = self.read(i);
                let on = value > *above;
                (
                    on == *want,
                    format!(
                        "{part} {}, i={} A",
                        if on { "on" } else { "off" },
                        number(value)
                    ),
                )
            }
            Bound::Level { pin, signal, want } => {
                let level = match self.engine.level(signal) {
                    Some(EngineLevel::High) => Level::High,
                    _ => Level::Low,
                };
                (level == *want, format!("{pin} {}", level.as_str()))
            }
            Bound::Compare {
                label,
                column,
                op,
                value,
                unit,
            } => {
                let seen = self.read(column);
                (
                    op.holds(seen, *value),
                    format!("{label} = {} {}", number(seen), unit.as_str()),
                )
            }
            Bound::SerialContains(text) => (
                self.serial.contains(text.as_str()),
                if self.serial.is_empty() {
                    "no serial".to_string()
                } else {
                    format!("serial {:?}", self.serial)
                },
            ),
            Bound::NoDiagnostics => {
                let found: Vec<Diagnostic> = self
                    .diagnostics()
                    .into_iter()
                    .filter(Diagnostic::runtime)
                    .collect();
                let codes: Vec<&str> = found.iter().map(|d| d.code).collect();
                (
                    found.is_empty(),
                    if codes.is_empty() {
                        "no diagnostics".to_string()
                    } else {
                        codes.join(", ")
                    },
                )
            }
        }
    }

    fn expect(&self, condition: &Condition) -> Outcome {
        let bound = self.bind(condition).map_err(fail)?;
        let (passed, actual) = self.observe(&bound);
        let check = Check {
            passed,
            expected: condition.to_string(),
            actual,
        };
        if passed {
            return Ok(Data::Checked(check));
        }
        let error = Error::new(
            Code::ExpectFailed,
            format!("expected {}", check.expected),
            "",
        );
        Err(failed(error, Data::Checked(check)))
    }

    fn step(&mut self, step: &Step) -> Outcome {
        let (length, until) = match step {
            Step::For(length) => (*length, None),
            Step::Until { condition, timeout } => (
                *timeout,
                Some((condition, self.bind(condition).map_err(fail)?)),
            ),
        };
        let from = self.engine.now().as_ps();
        let end = from.saturating_add(length.as_ps());
        let lit = |open: &Open| -> Vec<(String, bool)> {
            open.refs
                .top
                .iter()
                .filter_map(|p| Some((p.clone(), open.part(p)?)))
                .filter_map(|(p, r)| match r {
                    Reading::Part { on: Some(on), .. } => Some((p, on)),
                    _ => None,
                })
                .collect()
        };
        let mut was = lit(self);
        let mut changes = Vec::new();
        let mut more = 0;
        let mut met_at_ps = until
            .as_ref()
            .filter(|(_, bound)| self.observe(bound).0)
            .map(|_| from);
        let mut status = self.engine.status();
        let mut now = from;
        while met_at_ps.is_none() && now < end && status == Status::Running {
            let to = now.saturating_add(SLICE.as_ps()).min(end);
            status = self.engine.advance_to(Time::from_ps(to));
            now = self.engine.now().as_ps();
            let mut found = self.drain();
            let lit_now = lit(self);
            for ((part, before), (_, after)) in was.iter().zip(&lit_now) {
                if before != after {
                    found.push(Change::Led {
                        at_ps: now,
                        part: part.clone(),
                        on: *after,
                    });
                }
            }
            was = lit_now;
            if let Some((_, bound)) = &until
                && self.observe(bound).0
            {
                met_at_ps = Some(self.met_at(bound, &found, now));
            }
            for change in found {
                if changes.len() < CHANGES {
                    changes.push(change);
                } else {
                    more += 1;
                }
            }
        }
        let check = until.as_ref().map(|(condition, bound)| {
            let (passed, actual) = self.observe(bound);
            Check {
                passed,
                expected: condition.to_string(),
                actual,
            }
        });
        let data = Data::Stepped(Stepped {
            from_ps: from,
            met_at_ps,
            changes,
            more,
            check,
        });
        match status {
            Status::Faulted => Err(failed(self.faulted(), data)),
            Status::Stopped if met_at_ps.is_none() => Err(failed(
                Error::new(
                    Code::Ended,
                    "the session reached its 2-hour stop",
                    "run `open` to start again",
                ),
                data,
            )),
            _ if until.is_some() && met_at_ps.is_none() => Err(failed(
                Error::new(
                    Code::UntilTimeout,
                    format!("the condition did not hold within {length}"),
                    "step longer with `--timeout`, or check the circuit with `snapshot`",
                ),
                data,
            )),
            _ => Ok(data),
        }
    }

    /// An edge's own time for a pin level, else the slice's end.
    fn met_at(&self, bound: &Bound, found: &[Change], now: u64) -> u64 {
        let Bound::Level { pin, want, .. } = bound else {
            return now;
        };
        found
            .iter()
            .rev()
            .find_map(|c| match c {
                Change::Edge {
                    at_ps,
                    pin: p,
                    level,
                } if p == pin && level == want => Some(*at_ps),
                _ => None,
            })
            .unwrap_or(now)
    }

    /// The edges and serial since the last drain, which the session then forgets.
    fn drain(&mut self) -> Vec<Change> {
        let mut found = Vec::new();
        if let Some((edges, next)) = self.engine.edges_since(self.edges) {
            for edge in edges {
                found.push(Change::Edge {
                    at_ps: edge.at.as_ps(),
                    pin: self.signal_pins[edge.signal].clone(),
                    level: match edge.level {
                        EngineLevel::High => Level::High,
                        EngineLevel::Low => Level::Low,
                    },
                });
            }
            self.edges = next;
            self.engine.forget_edges(next);
        }
        if let Some((bytes, next)) = self.engine.serial_since(self.serial_cursor) {
            if let Some(first) = bytes.first() {
                let text: String = bytes.iter().map(|b| char::from(b.byte)).collect();
                found.push(Change::Serial {
                    at_ps: first.at.as_ps(),
                    text: text.clone(),
                });
                self.serial.push_str(&text);
                if self.serial.len() > SERIAL_KEPT {
                    let cut = self.serial.len() - SERIAL_KEPT;
                    let cut = (cut..self.serial.len())
                        .find(|&i| self.serial.is_char_boundary(i))
                        .unwrap_or(cut);
                    self.serial.drain(..cut);
                }
            }
            self.serial_cursor = next;
            self.engine.forget_serial(next);
        }
        found
    }

    fn snapshot(&self) -> Data {
        let mut owners: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for top in &self.refs.top {
            for net in self.refs.parts[top].pins.values().flatten() {
                owners.entry(net).or_default().insert(top);
            }
        }
        let wired = |net: &str| owners.get(net).is_some_and(|tops| tops.len() > 1);
        let mut parts = Vec::new();
        for top in &self.refs.top {
            let part = &self.refs.parts[top];
            let pins = part
                .pins
                .iter()
                .filter(|_| part.engine.is_empty())
                .filter_map(|(pin, net)| {
                    let net = net.as_deref().filter(|n| wired(n))?;
                    Some((pin.clone(), self.pin(pin, Some(net)).ok()?))
                })
                .collect();
            parts.push(PartState {
                name: top.clone(),
                ty: part.ty.clone(),
                value: self.part(top),
                pins,
            });
        }
        let lines: Vec<&str> = self.serial.lines().collect();
        let serial = lines[lines.len().saturating_sub(10)..].join("\n");
        Data::Snapshot(Snapshot {
            scene: self.scene.clone(),
            parts,
            serial,
            diagnostics: self.diagnostics(),
        })
    }
}
