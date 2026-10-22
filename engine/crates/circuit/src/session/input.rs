use sim_core::name::Name;
use sim_core::netlist::{Contact, Edge, Model};
use sim_core::time::Time;

use super::{Session, State, Status};

/// A change to one part, asked for from `at` on.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub part: Name,
    pub at: Time,
    pub setting: Setting,
}

/// What an input sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// A switch's contact.
    Contact(Contact),
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
    /// The part is not a switch.
    NotSettable,
    /// It would land at or after `stop`.
    PastStop,
    /// The setting is already in force where it would land.
    NoChange,
}

impl Session {
    /// Appends `input` to its part's events at the first time from the one it asks for that is
    /// after `now`'s accepted step, at or after `now` and the accepted step's reach, and legal
    /// after the part's last event. A batch run of the export takes every accepted step there.
    /// The held step is solved again with the new edge, and every step that then lands by
    /// `now` is accepted. The status is the session's after that re-solve. A fault ends the
    /// session at the accepted step.
    pub fn apply(&mut self, input: &Input) -> Result<Applied, Rejected> {
        let State::Running { accepted, now, .. } = &self.state else {
            return Err(Rejected::Ended);
        };
        let netlist = &self.circuit.netlist;
        let id = netlist.part_id(&input.part).ok_or(Rejected::UnknownPart)?;
        let (Model::Switch(switch), Setting::Contact(to)) =
            (&netlist.part(id).model, input.setting)
        else {
            return Err(Rejected::NotSettable);
        };
        let after_accepted = Time::from_ps(accepted.at.as_ps() + 1);
        let from = input.at.max(*now).max(after_accepted).max(accepted.reach);
        let at = switch
            .earliest(from)
            .filter(|&at| at < self.transient.stop)
            .ok_or(Rejected::PastStop)?;
        if *switch.schedule.after(at) == to {
            return Err(Rejected::NoChange);
        }
        self.circuit.netlist.append(id, Edge { at, to });
        self.breakpoints.arm(id, at);
        let State::Running { accepted, now, .. } = std::mem::replace(&mut self.state, State::TAKEN)
        else {
            unreachable!("the session runs");
        };
        self.state = self.solved(accepted);
        let status = self.advance_to(now);
        Ok(Applied { at, status })
    }

    /// The document `open_document` read, with every applied input in its part's `events`.
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
