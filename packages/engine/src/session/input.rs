use orbe_model::{Time, Value as FieldValue};

use crate::device::Mismatch;
use crate::netlist::{Appended, Baud, Contact, Frame, Model, Schedule, SerialSend, Value};
use crate::passive::position_rule;

use crate::diagnostic::ValueRule;
use crate::finite::Finite;
use crate::name::Name;

use super::{Session, State, Status};

/// A change to one part, asked for from `at` on.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub part: Name,
    pub at: Time,
    pub setting: Setting,
}

/// The bytes one `apply` sends on a `serial` source, as 8N1 frames at `baud`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Serial {
    pub baud: Baud,
    pub frame: Frame,
    pub bytes: Vec<u8>,
}

/// What an input sets. The part's kind decides which field it writes.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    /// A switch's contact.
    Contact(Contact),
    /// Volts, amperes, or the position fraction, in that field's SI unit.
    Level(Finite),
    /// 8N1 frames on a `serial` source.
    Serial(Serial),
    /// An event a device part declares, such as `touch` on a `cst816`.
    Event(DeviceEvent),
}

/// An event by the name its device declares, with a value for each of the event's fields in any
/// order. A `cst816` takes `touch` with integers `x` and `y` from 0 to 4095 and a bool `down`.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceEvent {
    pub name: String,
    pub fields: Vec<(String, FieldValue)>,
}

/// An input in its part's events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Where it landed, the time asked for or later.
    pub at: Time,
    /// The session's status after the re-solve.
    pub status: Status,
}

/// Why `apply` changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The session has stopped or faulted.
    Ended,
    /// No part has the input's name.
    UnknownPart,
    /// A setting on a part that takes none of any kind: a resistor, a capacitor, an inductor, a
    /// relay, a junction, a `step` or `pulse` source, or a chip. A device part takes only its
    /// events, so a contact, a level or a serial send on a `cst816` is this too.
    NotSettable,
    /// A contact on a source or a potentiometer, a level on a switch, a serial send on a
    /// switch, a potentiometer, or a plain or `steps` source, or a contact or a level on a
    /// `serial` source, or a device event on a switch, a potentiometer, a plain or `steps`
    /// source, or a `serial` source. On a device part, an event it does not declare, or one
    /// with a field missing, unknown, repeated or of the wrong kind.
    WrongSetting,
    /// A position outside `(0, 1)`, a leg that breaks the resistance rules, or a device event's
    /// integer outside its span, such as a touch coordinate outside 0 to 4095.
    Value(ValueRule),
    /// It would land at or after `stop`, or a serial send's frames would not end inside a
    /// time the engine can store.
    PastStop,
    /// The setting equals the value after the part's last event, or a serial send has no bytes.
    NoChange,
}

impl Session {
    /// Appends `input` to its part's events at the first time from the one it asks for that is
    /// after `now`'s accepted step, at or after `now` and the accepted step's reach, and legal
    /// after the part's last event. A contact sets a switch. A level sets a `steps` source or a
    /// potentiometer, in that field's unit. A serial send sets a `serial` source. It lands at the
    /// latest of that time and the end of the source's last event, so a send asked for while the
    /// last frames are still on the wire starts when the line is idle. The start bit is at that
    /// time. A device event lands on its part, and one equal to the last still lands.
    /// One call appends one event and solves the held step once, however many bytes it
    /// sends. A batch run of the export takes every accepted step there. The held step is solved
    /// again with the new edge, and every step that then lands by `now` is accepted. A level or a
    /// serial start is a jump, so that solve restarts. The status is the session's after that
    /// re-solve. A failed restart ends the session at the landing. Any other fault ends it at the
    /// accepted step.
    pub fn apply(&mut self, input: &Input) -> Result<Applied, Rejected> {
        let State::Running { accepted, now, .. } = &self.state else {
            return Err(Rejected::Ended);
        };
        let netlist = &self.circuit.netlist;
        let id = netlist.part_id(&input.part).ok_or(Rejected::UnknownPart)?;
        let after_accepted = Time::from_ps(accepted.at.as_ps() + 1);
        let from = input.at.max(*now).max(after_accepted).max(accepted.reach);
        let (appended, at, same) = decision(&netlist.part(id).model, &input.setting, from)?;
        if same {
            return Err(Rejected::NoChange);
        }
        let at = at
            .filter(|&at| at < self.transient.stop)
            .ok_or(Rejected::PastStop)?;
        self.circuit.netlist.append(id, at, appended);
        self.opened = None;
        self.breakpoints.arm(id, at);
        let State::Running { accepted, now, .. } = std::mem::replace(&mut self.state, State::TAKEN)
        else {
            unreachable!("the session runs");
        };
        self.state = self.solved(accepted);
        let status = self.advance_to(now);
        Ok(Applied { at, status })
    }

    /// Whether the session has a document for `export` to write, as `open_document` and
    /// `record_document` keep.
    pub fn has_document(&self) -> bool {
        self.document.is_some()
    }

    /// The document `open_document` read, with every applied input in its part. A switch event
    /// is appended to `events`. A plain `voltage`, `current` or `position` becomes `steps`, and
    /// a `steps` field gains the new levels. A `serial` field gains one event per applied send.
    /// A device part gains each applied event on its event's field, `touches` on a `cst816`.
    /// A part with nothing applied is unchanged. A plain source does not become a `serial` source.
    /// While the session runs, `stop_ps` is the held step's reach, at least `max_step_ps` and at
    /// most the document's `stop_ps`. No test pins the reach in that bound; the argument in `docs/engine/engine.md` Sessions covers it. A batch run of the export
    /// takes every step the session accepted and the one it holds. Once the session has ended
    /// `stop_ps` is the document's.
    /// `None` for a session `open` made.
    pub fn export(&self) -> Option<Vec<u8>> {
        let document = self.document.as_ref()?;
        let stop = match &self.state {
            State::Running { next, .. } => next
                .reach
                .max(Time::from_ps(self.transient.max_step))
                .min(self.transient.stop),
            State::Ended { .. } => self.transient.stop,
        };
        Some(document.export(&self.circuit.netlist, stop))
    }
}

/// The appended event, the first legal landing at or after `from`, and whether the setting
/// already equals the value after the part's last event.
fn decision(
    model: &Model,
    setting: &Setting,
    from: Time,
) -> Result<(Appended, Option<Time>, bool), Rejected> {
    match (model, setting) {
        (Model::Switch(switch), Setting::Contact(to)) => Ok((
            Appended::Contact(*to),
            switch.earliest(from),
            *switch.schedule.after_last() == *to,
        )),
        (Model::Switch(_), Setting::Level(_)) => Err(Rejected::WrongSetting),
        (Model::Potentiometer(pot), Setting::Level(level)) => {
            if let Some(rule) = position_rule(level.get(), pot.resistance.si.get()).first() {
                return Err(Rejected::Value(rule));
            }
            Ok(level_decision(&pot.position, *level, from))
        }
        (Model::Potentiometer(_), Setting::Contact(_)) => Err(Rejected::WrongSetting),
        (_, Setting::Event(event)) => event_decision(model, event, from),
        (_, Setting::Serial(send)) => serial_decision(model, send, from),
        (_, Setting::Contact(_)) => {
            if model.serial().is_some() || model.level().is_some() {
                Err(Rejected::WrongSetting)
            } else {
                Err(Rejected::NotSettable)
            }
        }
        (_, Setting::Level(level)) => {
            if model.serial().is_some() {
                return Err(Rejected::WrongSetting);
            }
            let Some(field) = model.level() else {
                return Err(Rejected::NotSettable);
            };
            Ok(level_decision(field.schedule, *level, from))
        }
    }
}

fn serial_decision(
    model: &Model,
    send: &Serial,
    from: Time,
) -> Result<(Appended, Option<Time>, bool), Rejected> {
    let Some(field) = model.serial() else {
        return Err(if takes_another_setting(model) {
            Rejected::WrongSetting
        } else {
            Rejected::NotSettable
        });
    };
    if send.bytes.is_empty() {
        return Err(Rejected::NoChange);
    }
    let at = match field.waveform.events().last() {
        Some(event) => from.max(event.end()),
        None => from,
    };
    let wire = SerialSend {
        baud: send.baud,
        frame: send.frame,
        bytes: send.bytes.clone(),
    };
    let at = wire.end(at).map(|_| at);
    Ok((Appended::Serial(wire), at, false))
}

fn takes_another_setting(model: &Model) -> bool {
    matches!(model, Model::Switch(_) | Model::Potentiometer(_))
        || model.level().is_some()
        || model.serial().is_some()
}

fn event_decision(
    model: &Model,
    event: &DeviceEvent,
    from: Time,
) -> Result<(Appended, Option<Time>, bool), Rejected> {
    let Some((row, events)) = model.device() else {
        return Err(if takes_another_setting(model) {
            Rejected::WrongSetting
        } else {
            Rejected::NotSettable
        });
    };
    let spec = row
        .events
        .iter()
        .find(|spec| spec.name == event.name)
        .ok_or(Rejected::WrongSetting)?;
    let declared = spec
        .declared(&event.fields)
        .map_err(|mismatch| match mismatch {
            Mismatch::Shape => Rejected::WrongSetting,
            Mismatch::Span => Rejected::Value(ValueRule::Span),
        })?;
    Ok((Appended::Event(declared), events.earliest(from), false))
}

fn level_decision(
    schedule: &Schedule<Value>,
    level: Finite,
    from: Time,
) -> (Appended, Option<Time>, bool) {
    (
        Appended::Level(level),
        schedule.earliest(from, 0),
        schedule.after_last().si == level,
    )
}
