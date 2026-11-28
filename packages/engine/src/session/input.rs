use orbe_model::{Time, Value as FieldValue};

use crate::device::Mismatch;
use crate::netlist::{
    Appended, Baud, Contact, Frame, MIN_GAP_PS, Model, Schedule, SerialSend, Value,
};
use crate::passive::position_rule;

use crate::diagnostic::ValueRule;
use crate::finite::Finite;
use crate::name::Name;

use super::{Session, State, Status};
use crate::tran::Changer;
use crate::world::Quantity;

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

/// A world quantity's value from `at` on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldChange {
    pub quantity: Quantity,
    pub at: Time,
    /// In the quantity's unit.
    pub value: Finite,
}

/// An input in its part's events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Where it landed, the time asked for or later.
    pub at: Time,
    /// The session's status after the re-solve.
    pub status: Status,
}

/// A world `set`: where it landed, the session afterwards, and how many of that quantity's
/// later scheduled steps it removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldApplied {
    /// Where it landed, the time asked for or later.
    pub at: Time,
    /// The session's status after the re-solve.
    pub status: Status,
    /// Later scheduled steps of this quantity that the set removed.
    pub dropped: u32,
    /// The value was already in force from the landing on. Later steps may still have been
    /// dropped. Nothing was appended.
    pub already: bool,
}

/// Why `apply` changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The session has stopped or faulted.
    Ended,
    /// No part has the input's name.
    UnknownPart,
    /// A setting on a part that takes none of any kind: a resistor, a capacitor, an inductor, a
    /// relay, a junction, a `step` or `pulse` source, a chip, or a device that declares no
    /// event. A device part takes only its events, so a contact, a level or a serial send on a
    /// `cst816` is this too.
    NotSettable,
    /// A contact on a source or a potentiometer, a level on a switch, a serial send on a
    /// switch, a potentiometer, or a plain or `steps` source, or a contact or a level on a
    /// `serial` source, or a device event on a switch, a potentiometer, a plain or `steps`
    /// source, or a `serial` source. On a device part, an event it does not declare, or one
    /// with a field missing, unknown, repeated or of the wrong kind.
    WrongSetting,
    /// A position outside `(0, 1)`, a leg that breaks the resistance rules, a device event's
    /// integer outside its span, such as a touch coordinate outside 0 to 4095, or a world value
    /// outside its quantity's range.
    Value(ValueRule),
    /// It would land at or after `stop`, or a serial send's frames would not end inside a
    /// time the engine can store.
    PastStop,
    /// The setting equals the value after the part's last event, or a serial send has no bytes.
    /// A world change equals the value in force, and that quantity has no later scheduled step.
    NoChange,
    /// The netlist does not declare the world quantity, so no part reads it and the trace has
    /// no column for it.
    Undeclared,
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
    /// sends. A batch run of the netlist with the input scheduled there takes every accepted
    /// step. The held step is solved again with the new edge, and every step that then lands by
    /// `now` is accepted. A level or a serial start is a jump, so that solve restarts. The status is the session's after that
    /// re-solve. A failed restart ends the session at the landing. Any other fault ends it at the
    /// accepted step.
    pub fn apply(&mut self, input: &Input) -> Result<Applied, Rejected> {
        let from = self.from(input.at)?;
        let netlist = &self.circuit.netlist;
        let id = netlist.part_id(&input.part).ok_or(Rejected::UnknownPart)?;
        let (appended, at, same) = decision(&netlist.part(id).model, &input.setting, from)?;
        if same {
            return Err(Rejected::NoChange);
        }
        let at = self.before_stop(at)?;
        self.circuit.netlist.append(id, at, appended);
        Ok(self.land(Changer::Part(id), at))
    }

    /// Lands `change` at the first time from the one it asks for that `apply` would land an
    /// input at, and drops that quantity's later steps. A step at the landing has not taken
    /// effect yet: a read there is the value from before it. Other quantities keep their steps.
    /// The value already in force from the landing on, when no later step differs from it, is
    /// [`Rejected::NoChange`]. A later step that differs is dropped and, when the value was
    /// already that, nothing is appended. An event exactly at the landing stays when it already
    /// holds the value. One that holds something else is dropped. When the value from before
    /// that event is the one asked for, nothing is appended: a read at the landing still sees
    /// it. A change of the value appends one step there. Either way the held step
    /// is solved again from the accepted one, as `apply` does, and a jump restarts. Every part
    /// field that reads the quantity follows the schedule this leaves.
    pub fn set(&mut self, change: &WorldChange) -> Result<WorldApplied, Rejected> {
        let from = self.from(change.at)?;
        let accepted_at = match &self.state {
            State::Running { accepted, .. } => accepted.at,
            State::Ended { .. } => unreachable!("from() accepted a running session"),
        };
        let quantity = change.quantity;
        let (at, keep_landing, append) = {
            let schedule = self
                .circuit
                .netlist
                .world()
                .schedule(quantity)
                .ok_or(Rejected::Undeclared)?;
            if !quantity.holds(change.value.get()) {
                return Err(Rejected::Value(ValueRule::Range));
            }
            let pending = schedule.after(from).si;
            let kept = schedule.before(from).si;
            let differs = schedule
                .events()
                .iter()
                .any(|event| event.at > from && event.to.si != change.value);
            if pending == change.value && !differs {
                return Err(Rejected::NoChange);
            }
            let at = if pending == change.value {
                from
            } else {
                landing_without_later(schedule, from).ok_or(Rejected::PastStop)?
            };
            // A step at `from` has not taken effect: a read there is `kept`. Keeping the step
            // is right only when it already holds the value. Dropping it restores `kept`, so a
            // set of that value appends nothing.
            let keep_landing = pending == change.value;
            let append = !keep_landing && kept != change.value;
            (at, keep_landing, append)
        };
        let at = self.before_stop(Some(at))?;
        // An event at the landing that already holds the value stays. A different one is
        // dropped, and replaced only when the value from before it differs.
        let drop_at = if keep_landing {
            Time::from_ps(from.as_ps().saturating_add(1))
        } else {
            from
        };
        let dropped = self.circuit.netlist.drop_world_from(quantity, drop_at);
        if append {
            self.circuit
                .netlist
                .append_world(quantity, at, change.value);
        }
        let next = self
            .circuit
            .netlist
            .world()
            .next_change_after(quantity, accepted_at);
        self.breakpoints.retarget(Changer::World(quantity), next);
        let applied = self.resettle(at);
        Ok(WorldApplied {
            at: applied.at,
            status: applied.status,
            dropped: u32::try_from(dropped).expect("a quantity's steps fit in the reply"),
            already: !append,
        })
    }

    /// The first time an input asked for at `asked` may land: after the accepted step, at or
    /// after `now`, and at or after the accepted step's reach.
    fn from(&self, asked: Time) -> Result<Time, Rejected> {
        let State::Running { accepted, now, .. } = &self.state else {
            return Err(Rejected::Ended);
        };
        let after_accepted = Time::from_ps(accepted.at.as_ps() + 1);
        Ok(asked.max(*now).max(after_accepted).max(accepted.reach))
    }

    fn before_stop(&self, at: Option<Time>) -> Result<Time, Rejected> {
        at.filter(|&at| at < self.transient.stop)
            .ok_or(Rejected::PastStop)
    }

    /// Arms `changer`'s new edge at `at` and solves the held step again from the accepted one.
    fn land(&mut self, changer: Changer, at: Time) -> Applied {
        self.breakpoints.arm(changer, at);
        self.resettle(at)
    }

    /// Solves the held step again from the accepted one. The breakpoints are already the
    /// schedule's.
    fn resettle(&mut self, at: Time) -> Applied {
        let State::Running { accepted, now, .. } = std::mem::replace(&mut self.state, State::TAKEN)
        else {
            unreachable!("the session runs");
        };
        self.state = self.solved(accepted);
        let status = self.advance_to(now);
        Applied { at, status }
    }
}

/// The first time at or after `from` that a new step can land once events at or after `from`
/// are gone. That is 1 ps with none left before `from`, and [`MIN_GAP_PS`] after the last one
/// that stays. `None` when that time does not fit in a time.
fn landing_without_later(schedule: &Schedule<Value>, from: Time) -> Option<Time> {
    let last = schedule.events().iter().rev().find(|event| event.at < from);
    let first = match last {
        None => 1,
        Some(event) => event.at.as_ps().checked_add(MIN_GAP_PS)?,
    };
    Some(Time::from_ps(from.as_ps().max(first)))
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
    matches!(model, Model::Switch(_)) || model.level().is_some() || model.serial().is_some()
}

fn event_decision(
    model: &Model,
    event: &DeviceEvent,
    from: Time,
) -> Result<(Appended, Option<Time>, bool), Rejected> {
    let Some((row, events)) = model.device().filter(|(row, _)| !row.events.is_empty()) else {
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
