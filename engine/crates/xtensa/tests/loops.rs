//! Zero-overhead loops on the flat test bus. Each program is GNU `as` output from `CODE`, and
//! each ends where the next fetch runs past the code, so a run that leaves the loop stops there.

use sim_xtensa::{
    Cause, Config, Core, Cycle, Decoded, Esp32, Esp32s3, Missing, Op, Sr, Stop, Unimplemented,
};

use crate::core::{CODE, Flat, RAM, core_at};

const WOE: u32 = 1 << 18;
const EXCM: u32 = 1 << 4;
const KERNEL: u32 = 0x4000_0300;

/// `loop a2, 1f; addi.n a3, a3, 1; 1: nop.n`
const COUNT: &str = "7682011b333df0";
/// `loop a2, 1f; beqz a4, 1f; addi.n a3, a3, 1; 1:`
const BRANCH: &str = "7682041614001b33";
/// `loop a2, 1f; addi.n a3, a3, 1; bnez a3, 2f; 1: nop.n; nop.n; 2:`
const TAKEN: &str = "7682041b335633003df03df0";
/// `loop a2, 1f; addi.n a3, a3, 1; l32i.n a5, a6, 0; 1:`
const FAULT: &str = "7682031b335806";
/// `loop a2, 1f; addi.n a3, a3, 1; call0 0x4037000c; 1:`, the call as bytes, since GNU `as`
/// refuses a call as a loop's last op.
const CALL: &str = "7682041b33450000";
/// `loop a2, 1f; addi.n a3, a3, 1; wsr.intset a5; 1:`
const INTSET: &str = "7682041b3350e213";

fn run(hex: &str, count: u32, setup: impl FnOnce(&mut Core<Esp32>)) -> (Stop, Core<Esp32>) {
    let mut bus = Flat::new(hex);
    let mut core: Core<Esp32> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_a(2, count);
    setup(&mut core);
    let stop = core.run_until(&mut bus, Cycle(10_000));
    (stop, core)
}

/// The first loop-back costs five cycles more, and a count of 1 has none.
#[test]
fn a_loop_runs_its_body_count_times() {
    for (count, cycles) in [(1, 3), (2, 9), (5, 12)] {
        let (stop, core) = run(COUNT, count, |_| {});
        assert_eq!(stop, Stop::Bus, "count {count}");
        assert_eq!(core.pc(), CODE + 7, "count {count}");
        assert_eq!(core.a(3), count, "count {count}");
        assert_eq!(core.special(Sr::Lcount), 0, "count {count}");
        assert_eq!(core.cycle(), Cycle(cycles), "count {count}");
    }
}

/// `LCOUNT` 0 at the loop op means 2^32 passes, so the body is still running at the limit.
#[test]
fn a_count_of_zero_keeps_looping() {
    let mut bus = Flat::new(COUNT);
    let mut core: Core<Esp32> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    assert_eq!(core.run_until(&mut bus, Cycle(200)), Stop::Limit);
    assert!(core.a(3) > 100);
    assert_eq!(core.a(3).wrapping_add(core.special(Sr::Lcount)), u32::MAX);
    assert_eq!(core.pc(), CODE + 3);
}

/// A taken branch to `LEND` leaves the loop with `LCOUNT` as it was (4.3.2.4, p 86).
#[test]
fn a_branch_to_lend_leaves_the_loop() {
    let (stop, core) = run(BRANCH, 4, |_| {});
    assert_eq!((stop, core.pc()), (Stop::Bus, CODE + 8));
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (0, 3));
    let (stop, core) = run(BRANCH, 4, |core| core.set_a(4, 1));
    assert_eq!((stop, core.pc()), (Stop::Bus, CODE + 8));
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (4, 0));
}

/// The ISA leaves `LCOUNT` undefined after a taken branch as the last op (4.3.2.2, p 86). This
/// core follows the fetch pseudo-code, which counts it down before the branch runs.
#[test]
fn a_taken_branch_as_the_last_op_still_counts_down() {
    let (stop, core) = run(TAKEN, 3, |_| {});
    assert_eq!((stop, core.pc()), (Stop::Bus, CODE + 12));
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (1, 1));
}

/// Pins a choice. The literal fetch pseudo-code (3.5.4.1, p 53, and 4.3.2.4, p 86) counts
/// `LCOUNT` down before a last op that raises an exception, and `Exception` (4.4.1.11, p 145)
/// does not restore it, so the rerun after the handler would lose a pass. This core takes the
/// precise reading, where the faulting op does not loop back and its rerun does. The S3 board
/// confirms it: an exception on the last op of a 3-pass loop's first pass leaves `LCOUNT` at 2,
/// and the rerun finishes all 3 passes. The classic chip is unprobed.
#[test]
fn pinned_choice_an_exception_in_the_last_op_loops_back_on_the_rerun() {
    let (stop, mut core) = run(FAULT, 3, |_| {});
    assert_eq!((stop, core.pc()), (Stop::Bus, KERNEL));
    assert_eq!(core.special(Sr::ExcCause), Cause::LoadProhibited as u32);
    assert_eq!(core.special(Sr::Epc1), CODE + 5);
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (1, 2));

    let mut bus = Flat::new(FAULT);
    core.set_special(Sr::Ps, WOE);
    core.set_a(6, RAM);
    core.set_pc(CODE + 5);
    assert_eq!(core.run_until(&mut bus, Cycle(10_000)), Stop::Bus);
    assert_eq!(core.pc(), CODE + 7);
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (3, 0));
}

/// A call as the last op stops when it would loop back, and runs when `LCOUNT` is 0.
#[test]
fn a_call_as_the_last_op_stops_at_the_loop_back() {
    let (stop, core) = run(CALL, 3, |_| {});
    let want = Unimplemented {
        pc: CODE + 5,
        missing: Missing::LoopEnd(Op::CALL0),
    };
    assert_eq!(stop, Stop::Unimplemented(want));
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (1, 2));
    assert_eq!(core.a(0), 0);

    let (stop, core) = run(CALL, 1, |_| {});
    assert_eq!((stop, core.pc()), (Stop::Bus, CODE + 12));
    assert_eq!(core.a(0), CODE + 8);
}

/// An interrupt the last op raises is taken at `LBEG`, after the loop-back.
#[test]
fn an_interrupt_after_the_last_op_returns_to_lbeg() {
    let (stop, core) = run(INTSET, 3, |core| {
        core.set_a(5, 1 << 7);
        core.set_special(Sr::IntEnable, 1 << 7);
    });
    assert_eq!((stop, core.pc()), (Stop::Bus, KERNEL));
    assert_eq!(core.special(Sr::ExcCause), Cause::Level1Interrupt as u32);
    assert_eq!(core.special(Sr::Epc1), CODE + 3);
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (1, 1));
}

/// `PS.EXCM` turns the loop-back off (4.3.2.3, p 86).
#[test]
fn exception_mode_turns_the_loop_back_off() {
    let (stop, core) = run(COUNT, 5, |core| core.set_special(Sr::Ps, WOE | EXCM));
    assert_eq!((stop, core.pc()), (Stop::Bus, CODE + 7));
    assert_eq!((core.a(3), core.special(Sr::Lcount)), (1, 4));
}

/// A level-1 timer between `loop` and the body still pays the first loop-back.
#[test]
fn a_vector_after_loop_keeps_the_first_loop_back() {
    vector_after_loop::<Esp32>();
    vector_after_loop::<Esp32s3>();
}

fn vector_after_loop<C: Config>() {
    let loop_op = [0x76u8, 0x82, 0x01];
    let addi = [0x1bu8, 0x33];
    let stop_op = [0x20u8, 0x63, 0x40];
    let wsr = [0x40u8, 0xf0, 0x13];
    let rfe = [0x00u8, 0x30, 0x00];
    let lend = CODE + 5;
    assert_eq!(
        C::render(&decode::<C>(&loop_op)),
        "loop\ta2, 0x40370005",
        "{}",
        C::NAME
    );
    assert_eq!(
        C::render(&decode::<C>(&addi)),
        "addi.n\ta3, a3, 1",
        "{}",
        C::NAME
    );
    assert_eq!(
        C::render(&decode::<C>(&wsr)),
        "wsr.ccompare0\ta4",
        "{}",
        C::NAME
    );
    assert_eq!(C::render(&decode::<C>(&rfe)), "rfe", "{}", C::NAME);

    let mut bus = Flat::new(&placed(&[
        (0, &loop_op),
        (3, &addi),
        (5, &stop_op),
        (0x300, &wsr),
        (0x303, &rfe),
    ]));
    let mut core: Core<C> = core_at(CODE);
    core.set_special(Sr::Ps, WOE);
    core.set_special(Sr::Vecbase, CODE);
    core.set_special(Sr::IntEnable, 1 << 6);
    core.set_special(Sr::Ccompare0, 1);
    core.set_a(2, 2);
    core.set_a(4, 0x1000);
    let stop = core.run_until(&mut bus, Cycle(100));
    let want = Unimplemented {
        pc: lend,
        missing: Missing::Op(Op::RER),
    };
    assert_eq!(stop, Stop::Unimplemented(want), "{}", C::NAME);
    assert_eq!(core.pc(), lend, "{}", C::NAME);
    assert_eq!(core.a(3), 2, "{}", C::NAME);
    assert_eq!(core.special(Sr::Lcount), 0, "{}", C::NAME);
    // loop, the vector, `wsr.ccompare0`, `rfe` plus its redirect, the first `addi.n`
    // (one refetch, the op, and the first loop-back), and the second `addi.n`.
    let cycles = 1 + 4 + 1 + 1 + C::redirect_cycles() + 1 + 1 + C::loopback_extra(0) + 1;
    assert_eq!(core.cycle(), Cycle(cycles), "{}", C::NAME);
}

fn decode<C: Config>(bytes: &[u8]) -> Decoded {
    C::decode(bytes, CODE).unwrap()
}

fn placed(parts: &[(usize, &[u8])]) -> String {
    let len = parts
        .iter()
        .map(|(at, bytes)| at + bytes.len())
        .max()
        .unwrap();
    let mut code = vec![0u8; len];
    for (at, bytes) in parts {
        code[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    code.iter().map(|byte| format!("{byte:02x}")).collect()
}
