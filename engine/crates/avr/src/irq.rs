//! Interrupt controller.
//!
//! The chip table lists every vector. A peripheral reports a line only when that
//! source's flag is set and its own enable bit is set. This module serves the
//! pending line with the lowest vector number when `SREG.I` is set.
//!
//! Entry finishes the current instruction, pushes the 16-bit return PC, clears
//! `I`, and jumps to the vector. That takes 4 cycles (DS40002061B §7.7.1).
//! Waking from `SLEEP` adds that many again. The mode's start-up delay runs
//! before this entry. The one-instruction delay after `SEI` and `RETI` is
//! DS40002198 `SEI` and DS40002061B §7.7: the next instruction runs before a
//! pending request is served.
//!
//! `MCUCR.IVSEL` (bit 1 of data address `0x55`) is a plain I/O byte. A write
//! is logged with the other unassigned I/O stores. The vectors stay at the
//! front of flash; boot-section placement is not modeled.
//!
//! Entry pushes two PC bytes. DS40002198 `RETI` for a 16-bit PC pops two
//! bytes, and flash ends at word `0x3FFF`.
//!
//! While the core is sleeping and no request is pending, time jumps to the
//! next scheduled event. The sleep mode decides which clocks run and how long
//! start-up takes; this module only adds the entry cycles.

use crate::chip::vector_word;

/// Cycles from the decision to serve a request until the vector instruction runs.
///
/// Waking from `SLEEP` adds the response time again. The mode-dependent startup
/// time is applied before entry and is not added here.
pub(crate) fn entry_cycles(response: u64, from_sleep: bool) -> u64 {
    if from_sleep {
        response.wrapping_add(response)
    } else {
        response
    }
}

/// Lowest vector number at or above 2 whose bit is set in `lines`.
///
/// `count` is the length of the chip's name table. The word address is
/// `2 × (number − 1)`.
pub(crate) fn select(lines: u64, count: usize) -> Option<(u8, u16)> {
    let mut bits = lines & !0b11;
    while bits != 0 {
        let number = bits.trailing_zeros() as u8;
        let index = usize::from(number.wrapping_sub(1));
        if number >= 1 && index < count {
            return Some((number, vector_word(number)));
        }
        bits &= !(1u64 << number);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{select, vector_word};
    use crate::Cpu;
    use crate::chip::VECTORS;

    fn insns(cpu: &mut Cpu, n: u32) {
        for _ in 0..n {
            if cpu.is_stopped() {
                break;
            }
            cpu.step();
        }
    }

    #[test]
    fn vector_word_is_two_bytes_per_number() {
        assert_eq!(vector_word(1), 0);
        assert_eq!(vector_word(2), 0x0002);
        assert_eq!(vector_word(17), 0x0020);
        assert_eq!(vector_word(43), 0x0054);
    }

    /// DS40002198 CALL: post-decrement. `[SP+1]` is PCH and `[SP+2]` is PCL.
    fn return_pc(cpu: &Cpu) -> u16 {
        let sp = u16::from(cpu.peek(0x005D)) | (u16::from(cpu.peek(0x005E)) << 8);
        let pch = cpu.peek(sp.wrapping_add(1));
        let pcl = cpu.peek(sp.wrapping_add(2));
        u16::from(pcl) | (u16::from(pch) << 8)
    }

    fn arm(cpu: &mut Cpu, vector: u8) {
        cpu.set_sreg(0x80);
        cpu.raise_irq_at(vector, 0);
    }

    #[test]
    fn lowest_vector_number_wins() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000]);
        arm(&mut cpu, 5);
        cpu.raise_irq_at(2, 0);
        cpu.step();
        assert_eq!(cpu.pc(), 0x0002);
        assert!(cpu.irq_acked(2));
        assert!(!cpu.irq_acked(5));
        assert_eq!(cpu.sreg() & 0x80, 0);
        assert!(select(0, VECTORS.len()).is_none());
        assert!(
            select(1 << 1, VECTORS.len()).is_none(),
            "RESET is not a request"
        );
    }

    #[test]
    fn i_flag_clear_masks_pending_requests() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000, 0x0000, 0x0000]);
        cpu.raise_irq_at(2, 0);
        insns(&mut cpu, 3);
        assert_eq!(cpu.pc(), 3);
        assert_eq!(cpu.cycles(), 3);
        assert_eq!(cpu.sreg() & 0x80, 0);
        assert!(!cpu.irq_acked(2));
    }

    #[test]
    fn entry_takes_four_cycles_on_atmega328p() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000]);
        arm(&mut cpu, 2);
        cpu.step();
        assert_eq!(cpu.cycles(), 5);
        assert_eq!(cpu.pc(), 0x0002);
        assert_eq!(cpu.peripheral_cycles(), 5);
    }

    #[test]
    fn entry_pushes_the_return_pc() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000, 0x0000]);
        arm(&mut cpu, 2);
        cpu.step();
        assert_eq!(return_pc(&cpu), 1);
        // DS40002198 CALL: SP ends at RAMEND − 2, [SP+1] = PCH, [SP+2] = PCL.
        assert_eq!(cpu.peek(0x005D), 0xFD);
        assert_eq!(cpu.peek(0x005E), 0x08);
        assert_eq!(cpu.peek(0x08FE), 0x00, "PCH");
        assert_eq!(cpu.peek(0x08FF), 0x01, "PCL");
    }

    #[test]
    fn skip_of_a_two_word_instruction_pushes_the_address_after_it() {
        // SBRS r0, 0 skips the JMP at words 1 and 2. The return PC is word 3.
        let mut cpu = Cpu::new();
        cpu.load_words(&[0xFE00, 0x940C, 0x0000, 0x0000]);
        cpu.set_reg(0, 1);
        arm(&mut cpu, 2);
        cpu.step();
        assert_eq!(return_pc(&cpu), 3);
        assert_eq!(cpu.pc(), 2);
    }

    #[test]
    fn entry_acks_the_vector() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000]);
        arm(&mut cpu, 17);
        cpu.step();
        assert_eq!(cpu.pc(), 0x0020);
        assert!(cpu.irq_acked(17));
        assert!(!cpu.irq_acked(2));
    }

    #[test]
    fn a_cleared_line_is_not_served() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000]);
        arm(&mut cpu, 2);
        cpu.clear_irq(2);
        cpu.step();
        assert_eq!(cpu.pc(), 1);
        assert_eq!(cpu.cycles(), 1);
        assert!(!cpu.irq_acked(2));
    }

    #[test]
    fn reti_runs_one_instruction_before_the_next_interrupt() {
        // Word 2 is INT0, so the handler is the RETI sitting on that vector.
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000, 0x9503, 0x9518]);
        arm(&mut cpu, 2);
        insns(&mut cpu, 3);
        assert_eq!(cpu.reg(16), 1, "the instruction after RETI ran");
        assert_eq!(cpu.pc(), 2);
        assert_eq!(return_pc(&cpu), 2);
        assert_eq!(cpu.cycles(), 1 + 4 + 4 + 1 + 4);
    }

    #[test]
    fn sei_runs_one_instruction_before_the_next_interrupt() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x9478, 0x9503, 0x0000]);
        cpu.raise_irq_at(2, 0);
        insns(&mut cpu, 2);
        assert_eq!(cpu.reg(16), 1, "the instruction after SEI ran");
        assert_eq!(cpu.pc(), 2);
        assert_eq!(return_pc(&cpu), 2);
        assert_eq!(cpu.sreg() & 0x80, 0);
    }

    #[test]
    fn a_multi_cycle_instruction_finishes_before_the_interrupt() {
        let mut cpu = Cpu::new();
        // ADIW r24, 1 is two cycles. The request latches on cycle 1 of it.
        cpu.load_words(&[0x9601]);
        cpu.set_sreg(0x80);
        cpu.raise_irq_at(2, 1);
        cpu.step();
        assert_eq!(cpu.reg(24), 1);
        assert_eq!(cpu.reg(25), 0);
        assert_eq!(return_pc(&cpu), 1);
        assert_eq!(cpu.pc(), 2);
        assert_eq!(cpu.cycles(), 2 + 4);
    }

    #[test]
    fn cli_does_not_serve_an_interrupt_that_arrives_with_it() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x94F8]);
        cpu.set_sreg(0x80);
        cpu.raise_irq_at(2, 1);
        cpu.step();
        assert_eq!(cpu.pc(), 1);
        assert_eq!(cpu.cycles(), 1);
        assert_eq!(cpu.sreg() & 0x80, 0);
        assert!(!cpu.irq_acked(2));
    }

    #[test]
    fn sleep_wakes_and_adds_four_cycles_on_atmega328p() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x9588, 0x9503]);
        // §10.11.1. SE must be one. SM is 000, Idle, so there is no start-up delay.
        cpu.poke(0x0053, 0x01);
        arm(&mut cpu, 2);
        cpu.step();
        assert!(!cpu.is_sleeping());
        assert_eq!(cpu.pc(), 2);
        assert_eq!(cpu.reg(16), 0);
        assert_eq!(return_pc(&cpu), 1);
        assert_eq!(cpu.cycles(), 1 + 4 + 4);
        assert_eq!(cpu.peripheral_cycles(), cpu.cycles());
        assert!(cpu.irq_acked(2));
    }

    #[test]
    fn a_cycle_budget_elapses_while_sleeping_with_nothing_pending() {
        // Idle, nothing pending. The budget is cycles, so time moves while
        // the core stays asleep.
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x9588, 0x9503]);
        cpu.poke(0x0053, 0x01);
        cpu.set_sreg(0x80);
        cpu.run_cycles(8);
        assert!(cpu.is_sleeping());
        assert_eq!(cpu.pc(), 1);
        assert_eq!(cpu.cycles(), 8);
        assert_eq!(cpu.reg(16), 0);
        assert_eq!(cpu.peripheral_cycles(), 8);
    }

    #[test]
    fn a_request_raised_while_asleep_wakes_on_the_next_run() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x9588]);
        cpu.poke(0x0053, 0x01);
        cpu.set_sreg(0x80);
        cpu.step();
        assert!(cpu.is_sleeping());
        assert_eq!(cpu.cycles(), 1);
        cpu.raise_irq_at(2, 0);
        cpu.run_cycles(1);
        assert!(!cpu.is_sleeping());
        assert_eq!(cpu.pc(), 2);
        assert_eq!(cpu.cycles(), 1 + 4 + 4);
        assert_eq!(return_pc(&cpu), 1);
    }

    #[test]
    fn sei_inside_a_handler_nests_after_one_instruction() {
        // Vector 3 is word 4. Vector 2 arrives while that handler runs.
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x0000, 0x0000, 0x0000, 0x0000, 0x9478, 0x9503, 0x0000]);
        arm(&mut cpu, 3);
        cpu.raise_irq_at(2, 6);
        insns(&mut cpu, 3);
        assert_eq!(cpu.reg(16), 1, "the instruction after SEI ran");
        assert_eq!(cpu.pc(), 2);
        assert_eq!(return_pc(&cpu), 6);
        assert!(cpu.irq_acked(3));
        assert!(cpu.irq_acked(2));
        assert_eq!(cpu.sreg() & 0x80, 0);
        // The outer frame was pushed first. Its PCL is at RAMEND.
        assert_eq!(cpu.peek(0x08FF), 1, "outer PCL");
        assert_eq!(cpu.peek(0x08FE), 0, "outer PCH");
    }

    #[test]
    fn ivsel_stays_a_byte_and_vectors_stay_put() {
        let mut cpu = Cpu::new();
        // OUT r16, MCUCR. IVSEL is bit 1. Data address 0x55, I/O address 0x35.
        cpu.set_reg(16, 0x02);
        cpu.load_words(&[0xBF05]);
        cpu.set_sreg(0x80);
        cpu.raise_irq_at(2, 0);
        cpu.step();
        assert_eq!(cpu.peek(0x0055), 0x02);
        assert_eq!(cpu.pc(), 2, "IVSEL did not move the vector");
    }

    #[test]
    fn peripherals_advance_by_the_cycles_just_spent() {
        let mut cpu = Cpu::new();
        cpu.load_words(&[0x9600, 0x0000]);
        cpu.step();
        assert_eq!(cpu.cycles(), 2);
        assert_eq!(cpu.peripheral_cycles(), 2);
        cpu.step();
        assert_eq!(cpu.cycles(), 3);
        assert_eq!(cpu.peripheral_cycles(), 3);
    }
}
