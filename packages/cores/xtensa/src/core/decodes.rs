//! Decoded instructions a core keeps, keyed by the PC, the length and the fetched bytes, and by
//! the bus's code version when it keeps one.
//!
//! The core fetches before it looks, and an encoding the decoder refuses is not stored. A copy
//! starts empty, so cores share nothing and a core stays `Send`. A bus with a code version lets
//! the core look before it fetches: a slot kept at the version the bus reads now holds what a
//! fetch would read, so the core runs it without fetching.

use crate::decode::{Insn, Op};

const SLOTS: usize = 65536;

const _: () = assert!(SLOTS.is_power_of_two());

#[derive(Clone)]
struct Slot {
    pc: u32,
    raw: u32,
    /// One more than the code version the slot was fetched at, or 0 when the bus keeps none.
    stamp: u64,
    insn: Insn,
}

const EMPTY: Slot = Slot {
    pc: 0,
    raw: 0,
    stamp: 0,
    insn: Insn {
        op: Op::NOP,
        len: 0,
        fields: [0; 8],
        field_count: 0,
        reach: 0,
    },
};

/// Allocated at the first fetch. Two tables always compare equal, since a table changes no
/// result.
#[derive(Default)]
pub(super) struct Decodes(Option<Box<[Slot]>>);

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
    /// The decode kept for `pc` at code version `version`, with no fetch.
    #[inline]
    pub(super) fn kept(&self, pc: u32, version: u64) -> Option<&Insn> {
        let slot = &self.0.as_ref()?[index(pc)];
        let stamp = version.wrapping_add(1);
        (slot.pc == pc && slot.stamp == stamp && stamp != 0 && slot.insn.len != 0)
            .then_some(&slot.insn)
    }

    /// The decode of the `len` bytes `raw` at `pc`, or `decode`'s, which the table then keeps
    /// with `version`. `decode` returning `None` stores nothing. A slot never filled has length
    /// 0, which no instruction has.
    pub(super) fn get_or(
        &mut self,
        pc: u32,
        len: u8,
        raw: u32,
        version: Option<u64>,
        decode: impl FnOnce() -> Option<Insn>,
    ) -> Option<Insn> {
        let slots = self
            .0
            .get_or_insert_with(|| vec![EMPTY; SLOTS].into_boxed_slice());
        let slot = &mut slots[index(pc)];
        if slot.pc == pc && slot.raw == raw && slot.insn.len == len {
            slot.stamp = stamp(version);
            return Some(slot.insn.clone());
        }
        let insn = decode()?;
        *slot = Slot {
            pc,
            raw,
            stamp: stamp(version),
            insn: insn.clone(),
        };
        Some(insn)
    }
}
