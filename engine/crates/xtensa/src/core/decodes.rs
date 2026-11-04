//! Decoded instructions a core keeps, keyed by the PC, the length and the fetched bytes.
//!
//! The core fetches before it looks, and an encoding the decoder refuses is not stored. A copy
//! starts empty, so cores share nothing and a core stays `Send`.

use crate::decode::{Insn, Op};

const SLOTS: usize = 65536;

const _: () = assert!(SLOTS.is_power_of_two());

#[derive(Clone)]
struct Slot {
    pc: u32,
    raw: u32,
    insn: Insn,
}

const EMPTY: Slot = Slot {
    pc: 0,
    raw: 0,
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

impl Decodes {
    /// The decode of the `len` bytes `raw` at `pc`, or `decode`'s, which the table then keeps.
    /// `decode` returning `None` stores nothing. A slot never filled has length 0, which no
    /// instruction has.
    pub(super) fn get_or(
        &mut self,
        pc: u32,
        len: u8,
        raw: u32,
        decode: impl FnOnce() -> Option<Insn>,
    ) -> Option<Insn> {
        let slots = self
            .0
            .get_or_insert_with(|| vec![EMPTY; SLOTS].into_boxed_slice());
        let slot = &mut slots[index(pc)];
        if slot.pc == pc && slot.raw == raw && slot.insn.len == len {
            return Some(slot.insn.clone());
        }
        let insn = decode()?;
        *slot = Slot {
            pc,
            raw,
            insn: insn.clone(),
        };
        Some(insn)
    }
}
