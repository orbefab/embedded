//! The host's replay engine. It sends a part's recorded transactions as records, each at its
//! end. The records are the part's netlist data, shared by every copy, and the engine keeps only
//! how far it has walked them. The device drives the part's pins.

use std::sync::Arc;

use orbe_model::{Drive, Time};

use super::native::{Hosting, Port};
use crate::link::Record;
use crate::world::World;

#[derive(Clone, Debug)]
pub(crate) struct Replay {
    /// In order of end. Each ends after the one before.
    records: Arc<[Record]>,
    /// The records before this one have ended where the engine reached.
    made: usize,
    /// The records before this one have left the engine.
    sent: usize,
}

impl Replay {
    pub(crate) fn new(records: Arc<[Record]>) -> Replay {
        debug_assert!(records.windows(2).all(|pair| pair[0].end < pair[1].end));
        Replay {
            records,
            made: 0,
            sent: 0,
        }
    }
}

impl<D: orbe_model::Device> Port<D> for Replay {
    const PINS: usize = 0;

    fn drive(&self, _image: &mut [Drive]) {}

    fn due_at(&self) -> Option<Time> {
        self.records.get(self.made).map(|record| record.end)
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, _world: &World, at: Time) -> bool {
        debug_assert_eq!(self.records[self.made].end, at);
        self.made += 1;
        true
    }

    fn input(&mut self, _dev: &mut Hosting<D>, _world: &World, _at: Time, levels: &[bool]) {
        assert!(levels.is_empty(), "a replay watches no pin");
    }

    fn records(&self) -> &[Record] {
        &self.records[self.sent..self.made]
    }

    fn drain_records(&mut self, through: Time, sent: &mut dyn FnMut(Record)) {
        while self.sent < self.made && self.records[self.sent].end <= through {
            sent(self.records[self.sent].clone());
            self.sent += 1;
        }
    }

    fn in_flight(&self, at: Time) -> Option<Record> {
        let record = self.records.get(self.made)?;
        debug_assert!(record.end > at, "a record that ended by `at` was made");
        (record.start <= at).then(|| record.clone())
    }
}
