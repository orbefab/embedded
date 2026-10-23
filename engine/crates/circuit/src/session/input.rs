use sim_core::diagnostic::ValueRule;
use sim_core::finite::Finite;
use sim_core::name::Name;
use sim_core::netlist::{Appended, Contact, Model, Schedule, Value, position_rule};
use sim_core::time::Time;

use super::{Session, State, Status};

/// A change to one part, asked for from `at` on.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub part: Name,
    pub at: Time,
    pub setting: Setting,
}

/// What an input sets. The part's kind decides which field it writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    /// A switch's contact.
    Contact(Contact),
    /// Volts, amperes, or the position fraction, in that field's SI unit.
    Level(Finite),
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
    /// A resistor, a capacitor, a junction, a chip, or a `step` or `pulse` source.
    NotSettable,
    /// A contact on a source or a potentiometer, or a level on a switch.
    WrongSetting,
    /// A position outside `(0, 1)`, or a leg that breaks the resistance rules.
    Value(ValueRule),
    /// It would land at or after `stop`.
    PastStop,
    /// The setting equals the value after the part's last event.
    NoChange,
}

impl Session {
    /// Appends `input` to its part's events at the first time from the one it asks for that is
    /// after `now`'s accepted step, at or after `now` and the accepted step's reach, and legal
    /// after the part's last event. A contact sets a switch. A level sets a `steps` source or a
    /// potentiometer, in that field's unit. A batch run of the export takes every accepted step
    /// there. The held step is solved again with the new edge, and every step that then lands by
    /// `now` is accepted. A level is a jump, so that solve restarts. The status is the session's
    /// after that re-solve. A fault ends the session at the accepted step.
    pub fn apply(&mut self, input: &Input) -> Result<Applied, Rejected> {
        let State::Running { accepted, now, .. } = &self.state else {
            return Err(Rejected::Ended);
        };
        let netlist = &self.circuit.netlist;
        let id = netlist.part_id(&input.part).ok_or(Rejected::UnknownPart)?;
        let after_accepted = Time::from_ps(accepted.at.as_ps() + 1);
        let from = input.at.max(*now).max(after_accepted).max(accepted.reach);
        let (appended, at, same) = decision(&netlist.part(id).model, input.setting, from)?;
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
    /// a `steps` field gains the new levels. A part with nothing applied is unchanged.
    /// While the session runs, `stop_ps` is the held step's reach, at least `max_step_ps` and at
    /// most the document's `stop_ps`. No test pins the reach in that bound; the argument in `docs/engine.md` Sessions covers it. A batch run of the export
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
    setting: Setting,
    from: Time,
) -> Result<(Appended, Option<Time>, bool), Rejected> {
    match (model, setting) {
        (Model::Switch(switch), Setting::Contact(to)) => Ok((
            Appended::Contact(to),
            switch.earliest(from),
            *switch.schedule.after_last() == to,
        )),
        (Model::Switch(_), Setting::Level(_)) => Err(Rejected::WrongSetting),
        (Model::Potentiometer(pot), Setting::Level(level)) => {
            if let Some(rule) = position_rule(level.get(), pot.resistance.si.get()).first() {
                return Err(Rejected::Value(rule));
            }
            Ok(level_decision(&pot.position, level, from))
        }
        (Model::Potentiometer(_), Setting::Contact(_)) => Err(Rejected::WrongSetting),
        (_, Setting::Contact(_)) => {
            if model.level().is_some() {
                Err(Rejected::WrongSetting)
            } else {
                Err(Rejected::NotSettable)
            }
        }
        (_, Setting::Level(level)) => {
            let Some(field) = model.level() else {
                return Err(Rejected::NotSettable);
            };
            Ok(level_decision(field.schedule, level, from))
        }
    }
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
