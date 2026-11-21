//! Decoded instructions a core keeps, keyed by the PC, the length and the fetched bytes, and by
//! the bus's code version when it keeps one.
//!
//! The core fetches before it looks, and an encoding the decoder refuses is not stored. A copy
//! starts empty, so cores share nothing and a core stays `Send`. A bus with a code version lets
//! the core look before it fetches: a slot kept at the version the bus reads now holds what a
//! fetch would read, so the core runs it without fetching.
//!
//! With each decode the table keeps what the step asks of the op every time it runs: its
//! coprocessor and whether it stores or divides. The core runs the op from its slot, so a
//! decode is never copied out.

use crate::config::Config;
use crate::decode::{Insn, Op};

const SLOTS: usize = 65536;

const _: () = assert!(SLOTS.is_power_of_two());

/// Whether an op that completes counts as a store for the next `memw`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stores {
    Never,
    Always,
    /// `s32c1i`, which stored when the old word it left in its `at` equals `SCOMPARE1`.
    Compared,
}

/// One decode and what the step asks of it.
#[derive(Clone)]
pub(super) struct Kept {
    pub(super) insn: Insn,
    /// The coprocessor whose `CPENABLE` bit the op needs.
    pub(super) coprocessor: Option<u8>,
    pub(super) stores: Stores,
    /// `quos`, `quou`, `rems` or `remu`, which the next divide's cost follows.
    pub(super) divides: bool,
}

impl Kept {
    fn new<C: Config>(insn: Insn) -> Kept {
        let op = insn.op;
        let stores = match op {
            Op::S8I | Op::S16I | Op::S32I | Op::S32I_N | Op::S32NB | Op::S32E | Op::S32RI => {
                Stores::Always
            }
            Op::SSI | Op::SSX | Op::SSIP | Op::SSXP | Op::st_QR => Stores::Always,
            Op::S32C1I => Stores::Compared,
            _ => Stores::Never,
        };
        Kept {
            coprocessor: C::coprocessor(op),
            stores,
            divides: matches!(op, Op::QUOU | Op::QUOS | Op::REMU | Op::REMS),
            insn,
        }
    }
}

#[derive(Clone)]
struct Slot {
    pc: u32,
    raw: u32,
    /// One more than the code version the slot was fetched at, or 0 when the bus keeps none.
    stamp: u64,
    kept: Kept,
}

const EMPTY: Slot = Slot {
    pc: 0,
    raw: 0,
    stamp: 0,
    kept: Kept {
        insn: Insn {
            op: Op::NOP,
            len: 0,
            fields: [0; 8],
            field_count: 0,
            reach: 0,
        },
        coprocessor: None,
        stores: Stores::Never,
        divides: false,
    },
};

/// The slots, allocated at the first fetch. Two tables always compare equal, since a table
/// changes no result.
#[derive(Default)]
pub(super) struct Decodes(Option<Box<[Slot]>>);

/// The slots, held apart from the core while it runs one of them.
pub(super) struct Taken(Box<[Slot]>);

impl Taken {
    #[inline]
    pub(super) fn kept(&self, slot: usize) -> &Kept {
        &self.0[slot].kept
    }
}

impl Clone for Decodes {
    fn clone(&self) -> Self {
        Decodes(None)
    }
}

impl PartialEq for Decodes {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Decodes {}

fn index(pc: u32) -> usize {
    ((pc ^ (pc >> 12)) as usize) & (SLOTS - 1)
}

/// The stamp of a slot fetched at `version`. A version of `u64::MAX` stamps 0, which no lookup
/// by version matches, so that slot is only ever found by its bytes.
fn stamp(version: Option<u64>) -> u64 {
    version.map_or(0, |version| version.wrapping_add(1))
}

impl Decodes {
    /// The slot that keeps `pc` at code version `version`, with no fetch.
    #[inline]
    pub(super) fn kept(&self, pc: u32, version: u64) -> Option<usize> {
        let at = index(pc);
        let slot = &self.0.as_ref()?[at];
        let stamp = version.wrapping_add(1);
        (slot.pc == pc && slot.stamp == stamp && stamp != 0 && slot.kept.insn.len != 0)
            .then_some(at)
    }

    /// The slot of the `len` bytes `raw` at `pc`, holding `decode`'s op when it did not hold
    /// these bytes, kept with `version`. `decode` returning `None` stores nothing. A slot never
    /// filled has length 0, which no instruction has.
    pub(super) fn get_or<C: Config>(
        &mut self,
        pc: u32,
        len: u8,
        raw: u32,
        version: Option<u64>,
        decode: impl FnOnce() -> Option<Insn>,
    ) -> Option<usize> {
        let slots = self
            .0
            .get_or_insert_with(|| vec![EMPTY; SLOTS].into_boxed_slice());
        let at = index(pc);
        let slot = &mut slots[at];
        if slot.pc == pc && slot.raw == raw && slot.kept.insn.len == len {
            slot.stamp = stamp(version);
            return Some(at);
        }
        let insn = decode()?;
        *slot = Slot {
            pc,
            raw,
            stamp: stamp(version),
            kept: Kept::new::<C>(insn),
        };
        Some(at)
    }

    /// The slots, which a fetch has allocated, until [`Decodes::put`] gives them back.
    #[inline]
    pub(super) fn take(&mut self) -> Taken {
        Taken(self.0.take().expect("a fetch allocated the table"))
    }

    #[inline]
    pub(super) fn put(&mut self, taken: Taken) {
        self.0 = Some(taken.0);
    }
}
