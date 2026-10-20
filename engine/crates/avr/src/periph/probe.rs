//! Interrupt source for the `irq` tests. It is not a device.

#[derive(Debug, Clone)]
pub(crate) struct Probe {
    /// Armed `(cycle, vector)` slots. `u64::MAX` is empty.
    armed: [(u64, u8); 4],
    pending: u64,
    acked: u64,
}

impl Probe {
    pub(crate) fn new() -> Self {
        Self {
            armed: [(u64::MAX, 0); 4],
            pending: 0,
            acked: 0,
        }
    }

    pub(crate) fn raise_at(&mut self, vector: u8, at_cycle: u64, now: u64) {
        assert!(vector < 64, "vector number fits in the request mask");
        if at_cycle <= now {
            self.pending |= 1u64 << vector;
            return;
        }
        let Some(slot) = self.armed.iter_mut().find(|slot| slot.0 == u64::MAX) else {
            panic!("interrupt test source schedule is full");
        };
        *slot = (at_cycle, vector);
    }

    pub(crate) fn clear(&mut self, vector: u8) {
        if vector < 64 {
            self.pending &= !(1u64 << vector);
        }
    }

    pub(crate) fn acked(&self, vector: u8) -> bool {
        vector < 64 && self.acked & (1u64 << vector) != 0
    }

    pub(crate) fn ack_vector(&mut self, vector: u8) {
        if vector < 64 {
            self.acked |= 1u64 << vector;
        }
    }

    pub(crate) fn pending_bits(&self) -> u64 {
        self.pending
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.armed
            .iter()
            .map(|slot| slot.0)
            .min()
            .unwrap_or(u64::MAX)
    }

    pub(crate) fn fire_at(&mut self, at: u64) {
        for slot in &mut self.armed {
            if slot.0 == at {
                self.pending |= 1u64 << slot.1;
                *slot = (u64::MAX, 0);
            }
        }
    }
}
