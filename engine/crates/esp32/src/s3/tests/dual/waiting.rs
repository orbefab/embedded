//! A core in `WAITI` while the other runs or waits too: each poll that can latch a change runs
//! at the cycle and tick it runs at when the waiting core polls after every step of the other.

use super::*;
use crate::s3::soc::{SPI2, SYSTIMER};
use crate::s3::spi2::TRANS_DONE;

const RFE: [u8; 3] = [0x00, 0x30, 0x00];
const SYSTIMER_TARGET0: u32 = 57;
const FROM_CPU_0: u32 = 79;
const SPI2_SOURCE: u32 = 21;
const CCOMPARE0_LINE: u32 = 1 << 6;
/// One second of SYSTIMER counts.
const SECOND: u32 = 16_000_000;

impl Program {
    /// Sets `VECBASE` from a5, enables `lines`, clears `PS` and waits in `WAITI`, then spins.
    fn wait_for(&mut self, lines: u8) -> &mut Program {
        self.wsr(5, VECBASE)
            .movi(6, lines)
            .wsr(6, INTENABLE)
            .movi(6, 0)
            .wsr(6, PS)
            .rsync()
            .raw(&WAITI)
            .raw(&LOOP)
    }

    /// With `SYSTEM` in a2 and the APP CPU's entry in a4, releases the APP CPU, then waits as
    /// [`Program::wait_for`] does.
    fn release_and_wait(&mut self, lines: u8) -> &mut Program {
        self.release().wait_for(lines)
    }
}

/// The PRO CPU releases the APP CPU into `app`, then waits in `WAITI` with interrupt 1
/// enabled and its vectors at `PRO_VECTORS`.
fn pro_waits(app: &Program) -> Program {
    let mut pro = Program::new(&[SYSTEM, app.entry(), PRO_VECTORS]);
    pro.l32r(2, 0).l32r(4, 1).l32r(5, 2).release_and_wait(2);
    pro
}

/// The APP CPU waits in `WAITI` with no interrupt enabled.
fn app_waits() -> Program {
    let mut app = Program::at(APP, &[]);
    app.raw(&WAITI).raw(&LOOP);
    app
}

/// Arms SYSTIMER alarm 0 on unit 0 to fire at `count`, or every `period` counts when `period`
/// is not 0, with its interrupt enabled and routed to interrupt 1 of `core`.
fn arm_alarm(chip: &mut Esp32s3, core: CoreId, count: u32, period: u32) {
    let systimer = &mut chip.soc.systimer;
    if period == 0 {
        systimer.write(Tick(0), 0x20, count);
    } else {
        systimer.write(Tick(0), 0x34, 1 << 30 | period);
    }
    systimer.write(Tick(0), 0x00, 0x4600_0000 | 1 << 24);
    systimer.write(Tick(0), 0x50, 1);
    systimer.write(Tick(0), 0x64, 1);
    let map = match core {
        CoreId::Pro => INTERRUPT_CORE0,
        CoreId::App => INTERRUPT_CORE1,
    };
    chip.soc.restore(map + 4 * SYSTIMER_TARGET0, 1);
}

fn systimer_due(chip: &mut Esp32s3) -> Tick {
    chip.soc.systimer.next_event(Tick(0)).expect("an alarm")
}

/// Each halt and the chip's tick after it, running to limits `step` crystal cycles apart up to
/// `through`.
fn ticks_after(mut chip: Esp32s3, through: u64, step: u64) -> Vec<(Halt, Tick)> {
    let mut seen = Vec::new();
    let mut limit = 0;
    while limit < through {
        limit = (limit + step).min(through);
        while let Some(halt) = chip.run_until(c(limit)) {
            seen.push((halt, chip.tick()));
        }
    }
    seen
}

/// The APP CPU stores to `FROM_CPU_0` after a delay with no store in it, while the PRO CPU
/// waits for that source on interrupt 1. The PRO CPU's handler raises GPIO21 at one tick
/// however the run splits.
#[test]
fn a_waiting_pro_cpu_wakes_at_the_store_that_raises_its_line() {
    let mut app = Program::at(APP, &[SYSTEM]);
    app.l32r(2, 0)
        .delay(5, 40)
        .movi(3, 1)
        .s32i(3, 2, 0x30)
        .raw(&LOOP);
    let pro = pro_waits(&app);
    let handler = pads_high(PRO_VECTORS + 0x300, GPIO21);
    let make = || {
        let mut chip = chip(&pro, &boot_rom(), &[&app, &handler]);
        chip.soc.restore(INTERRUPT_CORE0 + 4 * FROM_CPU_0, 1);
        chip
    };
    let seen = however_split(make, 400);
    assert_eq!(
        pad(&seen, 1),
        [(c(218), PadDrive::Low), (c(219), PadDrive::High)]
    );
}

/// The APP CPU waits for a SYSTIMER alarm while the PRO CPU runs a delay with no store. The
/// APP CPU's handler raises GPIO21 at one tick, the alarm's cycle and the handler's
/// instructions after it, however the run splits.
#[test]
fn a_waiting_app_cpu_takes_its_alarm_on_its_cycle_while_the_other_runs() {
    let mut app = Program::at(APP, &[APP_VECTORS]);
    app.l32r(5, 0).wait_for(2);
    let mut pro = Program::new(&[SYSTEM, app.entry()]);
    pro.l32r(2, 0).l32r(4, 1).release().delay(5, 250).raw(&LOOP);
    let handler = pads_high(APP_VECTORS + 0x300, GPIO21);
    let make = || {
        let mut chip = chip(&pro, &boot_rom(), &[&app, &handler]);
        arm_alarm(&mut chip, CoreId::App, 100, 0);
        chip
    };
    let seen = however_split(make, 800);
    assert_eq!(
        pad(&seen, 1),
        [(c(256), PadDrive::Low), (c(257), PadDrive::High)]
    );
}

/// The PRO CPU starts an SPI2 transfer with its end's interrupt enabled, then runs a delay with
/// no store, while the APP CPU waits for SPI2's line. The transfer sends the 8-bit command of the
/// reset value at a sixteenth of the crystal. The APP CPU's handler raises GPIO21 at one tick,
/// the transfer's end and the handler's instructions after it, however the run splits.
#[test]
fn a_waiting_app_cpu_takes_spi2s_end_on_its_cycle_while_the_other_runs() {
    let mut app = Program::at(APP, &[APP_VECTORS]);
    app.l32r(5, 0).wait_for(2);
    let (divide_by_16, update, usr) = (15 << 12, 1 << 23, 1 << 24);
    let literals = [
        SYSTEM,
        app.entry(),
        SPI2,
        divide_by_16,
        TRANS_DONE,
        update,
        usr,
    ];
    let mut pro = Program::new(&literals);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .delay(5, 40)
        .l32r(6, 2)
        .movi(7, 3)
        .s32i(7, 6, 0xe8)
        .l32r(7, 3)
        .s32i(7, 6, 0x0c)
        .l32r(7, 4)
        .s32i(7, 6, 0x34)
        .l32r(7, 5)
        .s32i(7, 6, 0x00)
        .l32r(7, 6)
        .s32i(7, 6, 0x00)
        .delay(5, 250)
        .raw(&LOOP);
    let handler = pads_high(APP_VECTORS + 0x300, GPIO21);
    let make = || {
        let mut chip = chip(&pro, &boot_rom(), &[&app, &handler]);
        chip.soc.restore(INTERRUPT_CORE1 + 4 * SPI2_SOURCE, 1);
        chip
    };
    let seen = however_split(make, 800);
    assert_eq!(
        seen.0[0].0,
        Halt::Change {
            at: c(373),
            image: false,
            record: true
        }
    );
    assert_eq!(
        pad(&seen, 1),
        [(c(379), PadDrive::Low), (c(380), PadDrive::High)]
    );
    assert_eq!(seen.1, Tick(9636));
}

/// The PRO CPU masks interrupts, lets its `CCOMPARE0` match latch, and only then runs
/// `WAITI 0`, which lets the latched interrupt in at once, while the APP CPU runs a delay with
/// no store. The handler raises GPIO21 at one tick however the run splits.
#[test]
fn a_core_takes_an_interrupt_latched_before_its_waiti_at_once() {
    let mut app = Program::at(APP, &[]);
    app.delay(5, 250).raw(&LOOP);
    let mut pro = Program::new(&[SYSTEM, app.entry(), PRO_VECTORS]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .l32r(5, 2)
        .wsr(5, VECBASE)
        .movi(6, CCOMPARE0_LINE as u8)
        .wsr(6, INTENABLE)
        .movi(6, 15)
        .wsr(6, PS)
        .rsync()
        .rsr(7, CCOUNT)
        .addi(7, 7, 20)
        .wsr(7, CCOMPARE0)
        .delay(8, 20)
        .raw(&WAITI)
        .raw(&LOOP);
    let handler = pads_high(PRO_VECTORS + 0x300, GPIO21);
    let seen = however_split(|| chip(&pro, &boot_rom(), &[&app, &handler]), 400);
    assert_eq!(
        pad(&seen, 1),
        [(c(101), PadDrive::Low), (c(102), PadDrive::High)]
    );
}

/// While one core waits, the other raises a pad, which halts the run, and then waits too. For
/// each core as the waiter, the chip's tick after each halt is the tick both cores had reached,
/// however the run splits.
#[test]
fn the_tick_after_a_halt_counts_a_waiting_core_where_it_would_have_polled() {
    for core in [CoreId::Pro, CoreId::App] {
        let (pro, app) = match core {
            CoreId::Pro => {
                let mut app = Program::at(APP, &[GPIO, GPIO4]);
                app.l32r(2, 0)
                    .l32r(3, 1)
                    .delay(5, 30)
                    .s32i(3, 2, 0x24)
                    .s32i(3, 2, 0x08)
                    .delay(5, 30)
                    .raw(&WAITI)
                    .raw(&LOOP);
                (pro_waits(&app), app)
            }
            CoreId::App => {
                let mut pro = Program::new(&[SYSTEM, APP, GPIO, GPIO21]);
                pro.l32r(2, 0)
                    .l32r(4, 1)
                    .release()
                    .l32r(6, 2)
                    .l32r(7, 3)
                    .delay(5, 50)
                    .s32i(7, 6, 0x24)
                    .s32i(7, 6, 0x08)
                    .raw(&WAITI)
                    .raw(&LOOP);
                (pro, app_waits())
            }
        };
        let make = || chip(&pro, &boot_rom(), &[&app]);
        let whole = ticks_after(make(), 400, 400);
        for step in [1, 2, 3, 7] {
            assert_eq!(
                ticks_after(make(), 400, step),
                whole,
                "{core:?}, steps of {step}"
            );
        }
        let expected = match core {
            CoreId::Pro => [(image_at(c(132)), c(133)), (image_at(c(133)), c(134))],
            CoreId::App => [(image_at(c(206)), c(207)), (image_at(c(207)), c(208))],
        };
        assert_eq!(whole, expected, "{core:?}");
    }
}

/// Both cores wait, and a SYSTIMER alarm at one second interrupts the PRO CPU. A run to 100 ms
/// returns with the chip past 100 ms and before the alarm, in one call or in steps of 1, 7 and
/// 37 ms. GPIO4 then rises from outside, and the PRO CPU's handler copies it to GPIO21 at the
/// alarm's tick and the handler's instructions after it.
#[test]
fn two_waiting_cores_reach_each_limit_and_take_a_pad_input() {
    let mut handler = Program::at(PRO_VECTORS + 0x300 - 12, &[GPIO, GPIO21, GPIO4]);
    handler
        .l32r(2, 0)
        .l32r(3, 1)
        .l32r(4, 2)
        .s32i(3, 2, 0x24)
        .l32i(5, 2, 0x3c)
        .and(5, 5, 4);
    let spin = handler.pc() + 6;
    handler.beqz(5, spin).s32i(3, 2, 0x08).raw(&LOOP);
    let app = app_waits();
    let pro = pro_waits(&app);
    let ms = |n: u64| Tick(n * TICK_HZ / 1000);
    let mut ticks = Vec::new();
    for step in [100, 1, 7, 37] {
        let mut chip = chip(&pro, &boot_rom(), &[&app, &handler]);
        chip.soc.gpio.write_io_mux(0x4 + 4 * 4, 0xa00);
        arm_alarm(&mut chip, CoreId::Pro, SECOND, 0);
        let alarm = systimer_due(&mut chip);
        let mut limit = 0;
        while limit < 100 {
            limit = (limit + step).min(100);
            assert_eq!(chip.run_until(ms(limit)), None, "steps of {step} ms");
            assert!(chip.tick() > ms(limit), "steps of {step} ms");
        }
        assert!(chip.tick() < alarm, "steps of {step} ms");
        ticks.push(chip.tick());
        chip.set_pad(4, true);
        let mut halts = Vec::new();
        while let Some(halt) = chip.run_until(ms(2000)) {
            halts.push((halt, chip.drive(21)));
        }
        assert_eq!(
            halts[..2],
            [
                (image_at(Tick(alarm.0 + 7 * XTAL)), PadDrive::Low),
                (image_at(Tick(alarm.0 + 11 * XTAL)), PadDrive::High),
            ],
            "steps of {step} ms"
        );
    }
    assert!(ticks.windows(2).all(|w| w[0] == w[1]), "{ticks:?}");
}

/// Both cores wait with nothing to wake either before timer group 0's watchdog runs out at
/// 500 ms. With the stage resetting the system, the run ends at its deadline. With the stage
/// interrupting the PRO CPU, the handler raises GPIO21 just after it.
#[test]
fn a_watchdog_fires_while_both_cores_wait() {
    let app = app_waits();
    let pro = pro_waits(&app);
    let handler = pads_high(PRO_VECTORS + 0x300, GPIO21);
    for action in [3, 1] {
        let mut chip = chip(&pro, &boot_rom(), &[&app, &handler]);
        let dogs = &mut chip.soc.watchdogs;
        dogs.write_timg(Tick(0), 0, 0x50, 20_000_000).unwrap();
        dogs.write_timg(Tick(0), 0, 0x48, EN | action << 29)
            .unwrap();
        chip.soc.restore(TIMG0 + 0x70, 1 << 2);
        chip.soc.restore(INTERRUPT_CORE0 + 4 * 52, 1);
        let expiry = chip.soc.watchdogs.deadline().expect("TIMG0 runs");
        let due = chip.soc.clock.round_up(expiry.at);
        assert_eq!(due, Tick(TICK_HZ / 2));
        let halt = chip.run_until(Tick(2 * TICK_HZ));
        if action == 3 {
            assert_eq!(halt, Some(Halt::Fault(due, TIMG0_FLASHBOOT)));
        } else {
            assert_eq!(halt, Some(image_at(Tick(due.0 + 6 * XTAL))));
            assert_eq!(
                chip.run_until(Tick(2 * TICK_HZ)),
                Some(image_at(Tick(due.0 + 7 * XTAL)))
            );
            assert_eq!(chip.drive(21), PadDrive::High);
        }
    }
}

/// Both cores wait, and a SYSTIMER alarm every 10 ms interrupts the PRO CPU, whose handler
/// counts it and returns to `WAITI`. Returns how many it counted in a run of `seconds`, which
/// fails past 5 slices for each 10 ms and 100 more.
fn alarms_taken(seconds: u32) -> u32 {
    let mut handler = Program::at(PRO_VECTORS + 0x300 - 8, &[SYSTIMER, DATA]);
    handler
        .l32r(2, 0)
        .movi(3, 1)
        .s32i(3, 2, 0x6c)
        .l32r(4, 1)
        .l32i(5, 4, 0)
        .addi(5, 5, 1)
        .s32i(5, 4, 0)
        .raw(&RFE);
    let app = app_waits();
    let mut pro = Program::new(&[SYSTEM, app.entry(), PRO_VECTORS]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .l32r(5, 2)
        .release()
        .wsr(5, VECBASE)
        .movi(6, 2)
        .wsr(6, INTENABLE)
        .movi(6, 0)
        .wsr(6, PS)
        .rsync();
    let top = pro.pc();
    pro.raw(&WAITI).j(top);
    let mut chip = chip(&pro, &boot_rom(), &[&app, &handler]);
    arm_alarm(&mut chip, CoreId::Pro, 0, SECOND / 100);
    chip.cores.slices_left = 500 * u64::from(seconds) + 100;
    assert_eq!(chip.run_until(Tick(u64::from(seconds) * TICK_HZ)), None);
    chip.soc.bus().load(Cycle(0), DATA, Width::Word).unwrap()
}

/// Both cores wait through 1 s and 1000 s of 10 ms alarms, and the PRO CPU takes every one,
/// with each core asleep between them.
#[test]
fn two_waiting_cores_skip_to_the_next_alarm() {
    assert_eq!(alarms_taken(1), 99);
    assert_eq!(alarms_taken(1000), 99_999);
}

/// The PRO CPU switches the clock from the crystal to 240 MHz with a straddling store while
/// the APP CPU waits for its `CCOMPARE0` match, set to fall `wait` cycles after it starts
/// waiting, before or after the switch. The match's handler toggles GPIO4 every cycle, and the
/// PRO CPU then toggles GPIO21 every cycle. Each pad's changes come at rising ticks however
/// the run splits.
#[test]
fn a_clock_switch_leaves_a_waiting_core_on_the_axis() {
    for wait in [4, 40, 120] {
        let mut app = Program::at(APP, &[APP_VECTORS]);
        app.l32r(2, 0)
            .wsr(2, VECBASE)
            .movi(6, CCOMPARE0_LINE as u8)
            .wsr(6, INTENABLE)
            .movi(6, 0)
            .wsr(6, PS)
            .rsr(7, CCOUNT)
            .addi(7, 7, wait)
            .wsr(7, CCOMPARE0)
            .raw(&WAITI)
            .raw(&LOOP);
        let handler = toggles(APP_VECTORS + 0x300 - 8, 40);
        let mut pro = Program::new(&[SYSTEM, app.entry(), 0x400, GPIO, GPIO21]);
        pro.l32r(2, 0)
            .l32r(4, 1)
            .movi(6, 0xe)
            .s32i(6, 2, 0x10)
            .l32r(7, 2)
            .release()
            .delay(8, 10)
            .straddling_s32i(7, 2, 0x60)
            .l32r(9, 3)
            .l32r(10, 4)
            .s32i(10, 9, 0x24);
        for _ in 0..40 {
            pro.s32i(10, 9, 0x08).s32i(10, 9, 0x0c);
        }
        pro.raw(&LOOP);
        let seen = however_split(|| chip(&pro, &boot_rom(), &[&app, &handler]), 150);
        for k in [0, 1] {
            let changes = pad(&seen, k);
            assert!(changes.len() > 40, "wait {wait} pad {k}: {changes:?}");
            assert!(
                changes.windows(2).all(|w| w[0].0 < w[1].0),
                "wait {wait} pad {k}: {changes:?}"
            );
        }
    }
}

/// The APP CPU sets its `CCOMPARE0` match and waits, and the PRO CPU gates its clock for a
/// stretch of the wait. `CCOUNT` does not count the gated stretch, so the match, and the
/// handler's GPIO21, come that much later, at one tick however the run splits.
#[test]
fn a_gated_clock_delays_a_waiting_cores_timer_match() {
    let mut app = Program::at(APP, &[APP_VECTORS]);
    app.l32r(2, 0)
        .wsr(2, VECBASE)
        .movi(6, CCOMPARE0_LINE as u8)
        .wsr(6, INTENABLE)
        .movi(6, 0)
        .wsr(6, PS)
        .rsr(7, CCOUNT)
        .addi(7, 7, 100)
        .wsr(7, CCOMPARE0)
        .raw(&WAITI)
        .raw(&LOOP);
    let handler = pads_high(APP_VECTORS + 0x300, GPIO21);
    let mut pro = Program::new(&[SYSTEM, app.entry()]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .delay(5, 20)
        .movi(3, 0)
        .s32i(3, 2, 0)
        .delay(5, 30)
        .movi(3, 2)
        .s32i(3, 2, 0)
        .raw(&LOOP);
    let seen = however_split(|| chip(&pro, &boot_rom(), &[&app, &handler]), 400);
    assert_eq!(
        pad(&seen, 1),
        [(c(243), PadDrive::Low), (c(244), PadDrive::High)]
    );
}

/// While one core waits, the other makes a straddling store that turns on an interrupt the
/// model never raises, which ends the run a cycle after its step's start. The chip's tick after
/// the fault is the cycle the waiting core would have reached polling after each step of the
/// other, however the run splits.
#[test]
fn the_tick_after_a_fault_counts_a_waiting_core_where_it_would_have_polled() {
    let refuses = |program: &mut Program| {
        program
            .movi(3, 1)
            .delay(5, 30)
            .straddling_s32i(3, 6, 0x70)
            .raw(&LOOP);
    };
    for core in [CoreId::Pro, CoreId::App] {
        let (pro, app) = match core {
            CoreId::Pro => {
                let mut app = Program::at(APP, &[TIMG0]);
                app.l32r(6, 0);
                refuses(&mut app);
                (pro_waits(&app), app)
            }
            CoreId::App => {
                let mut pro = Program::new(&[SYSTEM, APP, TIMG0]);
                pro.l32r(2, 0).l32r(4, 1).release().l32r(6, 2);
                refuses(&mut pro);
                (pro, app_waits())
            }
        };
        let make = || chip(&pro, &boot_rom(), &[&app]);
        let whole = ticks_after(make(), 400, 400);
        for step in [1, 2, 3, 7] {
            assert_eq!(
                ticks_after(make(), 400, step),
                whole,
                "{core:?}, steps of {step}"
            );
        }
        let ticks: Vec<(Tick, Tick)> = whole
            .iter()
            .map(|&(halt, tick)| (halt.at(), tick))
            .collect();
        let expected = match core {
            CoreId::Pro => [(c(136), c(136))],
            CoreId::App => [(c(130), c(129))],
        };
        assert_eq!(ticks, expected, "{core:?}");
        assert!(
            matches!(
                whole[0].0,
                Halt::Fault(_, Fault::Refused(Refused::Interrupt("TIMG0"), _))
            ),
            "{core:?}: {whole:?}"
        );
    }
}
