//! Both cores: the APP CPU's start, hold and reset, the strict order across a split run, and
//! the time axis they share. Each chip has a ROM of a few instructions at the reset vector.

use super::super::rom;
use super::*;

const APP: u32 = BASE + 0x1000;
const PRO_VECTORS: u32 = BASE + 0x2000;
const APP_VECTORS: u32 = BASE + 0x2400;
const DATA: u32 = 0x3fcb_0000;
const RESET: u32 = 0x4000_0400;
const INTERRUPT_CORE0: u32 = 0x600c_2000;
const INTERRUPT_CORE1: u32 = INTERRUPT_CORE0 + 0x800;
const GPIO4: u32 = 1 << 4;

const VECBASE: u8 = 231;
const CCOUNT: u8 = 234;
const CCOMPARE0: u8 = 240;

impl Program {
    fn s32i(&mut self, t: u8, s: u8, offset: u8) -> &mut Program {
        self.rri8(6, t, s, offset)
    }

    fn l32i(&mut self, t: u8, s: u8, offset: u8) -> &mut Program {
        self.rri8(2, t, s, offset)
    }

    fn nop(&mut self) -> &mut Program {
        self.raw(&[0xf0, 0x20, 0x00])
    }

    fn rsync(&mut self) -> &mut Program {
        self.raw(&[0x10, 0x20, 0x00])
    }

    fn rsr(&mut self, t: u8, sr: u8) -> &mut Program {
        self.raw(&[t << 4, sr, 0x03])
    }

    fn addi(&mut self, t: u8, s: u8, imm: i8) -> &mut Program {
        self.raw(&[t << 4 | 2, 0xc0 | s, imm as u8])
    }

    /// `and ar, as, at`.
    fn and(&mut self, r: u8, s: u8, t: u8) -> &mut Program {
        self.raw(&[t << 4, r << 4 | s, 0x10])
    }

    fn branch(&mut self, op: u8, s: u8, to: u32) -> &mut Program {
        let imm = to.wrapping_sub(self.pc() + 4);
        self.raw(&[op, s | (imm as u8 & 0xf) << 4, (imm >> 4) as u8])
    }

    fn beqz(&mut self, s: u8, to: u32) -> &mut Program {
        self.branch(0x16, s, to)
    }

    fn bnez(&mut self, s: u8, to: u32) -> &mut Program {
        self.branch(0x56, s, to)
    }

    fn j(&mut self, to: u32) -> &mut Program {
        let offset = to.wrapping_sub(self.pc() + 4);
        self.raw(&[
            0x06 | (offset as u8 & 3) << 6,
            (offset >> 2) as u8,
            (offset >> 10) as u8,
        ])
    }

    fn jx(&mut self, s: u8) -> &mut Program {
        self.raw(&[0xa0, s, 0x00])
    }

    /// `n` passes of a two-op loop on `at`.
    fn delay(&mut self, t: u8, n: u8) -> &mut Program {
        self.movi(t, n);
        let top = self.pc();
        self.addi(t, t, -1).bnez(t, top)
    }

    /// A `j` to a 3-byte `s32i at, as, offset` two bytes past a word boundary, which lands its
    /// store a cycle after its step's start.
    fn straddling_s32i(&mut self, t: u8, s: u8, offset: u8) -> &mut Program {
        let target = (self.pc() + 3 + 4) & !3 | 2;
        self.j(target);
        while self.pc() < target {
            self.raw(&[0]);
        }
        self.s32i(t, s, offset)
    }

    /// With `SYSTEM` in a2: the boot address `entry` from a4 into `CONTROL_1`, then
    /// `CONTROL_0` from power-on 0x4 to `CLKGATE_EN`, which releases the APP CPU's reset.
    fn release(&mut self) -> &mut Program {
        self.s32i(4, 2, 4).movi(3, 2).s32i(3, 2, 0)
    }
}

/// The ROM's reset path, cut short: poll `SYSTEM_CORE_1_CONTROL_1_REG` and jump to it.
fn boot_rom() -> Program {
    let mut rom = Program::at(RESET - 4, &[SYSTEM + 4]);
    rom.l32r(2, 0);
    let poll = rom.pc();
    rom.l32i(3, 2, 0).beqz(3, poll).jx(3);
    rom
}

/// At `at`, sets the pad bits in `bits` high and spins.
fn pads_high(at: u32, bits: u32) -> Program {
    let mut handler = Program::at(at - 8, &[GPIO, bits]);
    handler
        .l32r(2, 0)
        .l32r(3, 1)
        .s32i(3, 2, 0x24)
        .s32i(3, 2, 0x08)
        .raw(&LOOP);
    handler
}

/// Enables GPIO4 and toggles it `n` times, one store a cycle, then spins.
fn toggles(at: u32, n: usize) -> Program {
    let mut app = Program::at(at, &[GPIO, GPIO4]);
    app.l32r(2, 0).l32r(3, 1).s32i(3, 2, 0x24);
    for _ in 0..n {
        app.s32i(3, 2, 0x08).s32i(3, 2, 0x0c);
    }
    app.raw(&LOOP);
    app
}

/// A chip that runs `pro` on the PRO CPU and has `rom` in its ROM, with `more` in SRAM too,
/// GPIO4 and GPIO21 netted and every watchdog off.
fn chip(pro: &Program, rom: &Program, more: &[&Program]) -> Esp32s3 {
    let images: Vec<(u32, Vec<u8>)> = [pro]
        .into_iter()
        .chain(more.iter().copied())
        .map(|program| (program.base, program.image()))
        .collect();
    let segments = images.iter().map(|(at, bytes)| (*at, bytes.as_slice()));
    let mut chip = Esp32s3::new(&[4, 21], pro.entry(), segments, None);
    let mut bytes = vec![0u8; (rom::ROM.end - rom::ROM.start) as usize];
    let at = (rom.base - rom::ROM.start) as usize;
    let image = rom.image();
    bytes[at..at + image.len()].copy_from_slice(&image);
    chip.soc.attach_rom(bytes.into());
    chip.soc.watchdogs.write_timg(Tick(0), 0, 0x48, 0).unwrap();
    chip.soc.watchdogs.write_rtc(Tick(0), 0x98, 0).unwrap();
    chip
}

type Seen = (Vec<(Halt, [PadDrive; 2])>, Tick);

/// Each halt with GPIO4's and GPIO21's drives after it, and the final tick, from runs to each
/// crystal cycle `step` apart through `through`.
fn seen(mut chip: Esp32s3, through: u64, step: u64) -> Seen {
    let mut seen = Vec::new();
    let mut limit = 0;
    while limit < through {
        limit = (limit + step).min(through);
        while let Some(halt) = chip.run_until(c(limit)) {
            seen.push((halt, [chip.drive(4), chip.drive(21)]));
        }
    }
    (seen, chip.tick())
}

/// What one run to `through` shows, which runs split 1, 2, 3 and 7 cycles apart also show.
fn however_split(make: impl Fn() -> Esp32s3, through: u64) -> Seen {
    let whole = seen(make(), through, through);
    for step in [1, 2, 3, 7] {
        assert_eq!(seen(make(), through, step), whole, "steps of {step} cycles");
    }
    whole
}

/// The ticks at which pad `k` of [GPIO4, GPIO21] changed, with the drive it took.
fn pad(seen: &Seen, k: usize) -> Vec<(Tick, PadDrive)> {
    let mut last = PadDrive::Off;
    let mut changes = Vec::new();
    for (halt, drives) in &seen.0 {
        if drives[k] != last {
            last = drives[k];
            changes.push((halt.at(), last));
        }
    }
    changes
}

fn app(pc: u32) -> Pc {
    Pc {
        core: CoreId::App,
        pc,
    }
}

/// The PRO CPU releases the APP CPU, runs three instructions, and stores the boot address the
/// APP CPU's ROM polls for. Each poll sits at one place in the order however the run splits, so
/// the APP CPU reaches its code, and raises GPIO21, at one tick.
#[test]
fn a_boot_address_stored_just_after_the_release_runs_the_same_however_the_run_splits() {
    let mut pro = Program::new(&[SYSTEM, APP + 8]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .movi(3, 2)
        .s32i(3, 2, 0)
        .nop()
        .nop()
        .nop()
        .s32i(4, 2, 4)
        .raw(&LOOP);
    let app_code = pads_high(APP + 8, GPIO21);
    let rom = boot_rom();
    let seen = however_split(|| chip(&pro, &rom, &[&app_code]), 200);
    let spin = app_code.entry() + 12;
    assert_eq!(
        seen.0,
        [
            (image_at(c(16)), [PadDrive::Off, PadDrive::Low]),
            (image_at(c(17)), [PadDrive::Off, PadDrive::High]),
            (
                Halt::Fault(c(18), Fault::Halted(app(spin))),
                [PadDrive::Off, PadDrive::High]
            ),
        ]
    );
}

/// The APP CPU stalls itself before a store to `FROM_CPU_0`, which the PRO CPU's map routes to
/// a level-4 line. The PRO CPU's store that clears `RUNSTALL` comes a cycle before its
/// `CCOMPARE0` level-1 interrupt is due. The APP CPU's store runs before the PRO CPU's next
/// poll, which takes level 4 first, and its handler raises GPIO21. The level-1 handler would
/// raise GPIO4.
#[test]
fn a_resumed_app_cpu_runs_before_the_storers_next_poll() {
    let mut pro = Program::new(&[
        SYSTEM,
        APP + 4,
        INTERRUPT_CORE0 + 4 * 79,
        PRO_VECTORS,
        1 << 24 | 1 << 6,
    ]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .l32r(5, 2)
        .movi(6, 24)
        .s32i(6, 5, 0)
        .l32r(5, 3)
        .wsr(5, VECBASE)
        .l32r(6, 4)
        .wsr(6, INTENABLE)
        .movi(6, 0)
        .wsr(6, PS)
        .rsync()
        .movi(8, 1);
    let held = pro.pc();
    pro.l32i(6, 2, 0)
        .and(9, 6, 8)
        .beqz(9, held)
        .rsr(7, CCOUNT)
        .addi(7, 7, 4)
        .wsr(7, CCOMPARE0)
        .s32i(3, 2, 0)
        .raw(&LOOP);
    let mut app_code = Program::at(APP, &[SYSTEM]);
    app_code
        .l32r(4, 0)
        .movi(6, 1)
        .movi(5, 3)
        .s32i(5, 4, 0)
        .s32i(6, 4, 0x30)
        .raw(&LOOP);
    let level4 = pads_high(PRO_VECTORS + 0x200, GPIO21);
    let level1 = pads_high(PRO_VECTORS + 0x300, GPIO4);
    let rom = boot_rom();
    let seen = however_split(|| chip(&pro, &rom, &[&app_code, &level4, &level1]), 300);
    assert_eq!(pad(&seen, 0), []);
    assert_eq!(
        pad(&seen, 1),
        [(c(29), PadDrive::Low), (c(30), PadDrive::High)]
    );
}

/// The APP CPU maps `FROM_CPU_1` to its CPU interrupt 2 and waits in `WAITI`. The PRO CPU's
/// store to `FROM_CPU_1` interrupts it, and its handler raises GPIO21 at one tick however the
/// run splits. With the APP CPU's map left at 16, a core timer, nothing reaches it.
#[test]
fn a_from_cpu_store_interrupts_the_app_cpu() {
    let mut pro = Program::new(&[SYSTEM, APP + 12, DATA]);
    pro.l32r(2, 0).l32r(4, 1).release().l32r(5, 2);
    let wait = pro.pc();
    pro.l32i(6, 5, 0)
        .beqz(6, wait)
        .movi(7, 1)
        .s32i(7, 2, 0x34)
        .raw(&LOOP);
    let app_code = |mapped: bool| {
        let mut app = Program::at(APP, &[INTERRUPT_CORE1 + 4 * 80, APP_VECTORS, DATA]);
        if mapped {
            app.l32r(2, 0).movi(3, 2).s32i(3, 2, 0);
        } else {
            app.nop().nop().nop();
        }
        app.l32r(2, 1)
            .wsr(2, VECBASE)
            .movi(3, 4)
            .wsr(3, INTENABLE)
            .movi(3, 0)
            .wsr(3, PS)
            .rsync()
            .l32r(2, 2)
            .movi(3, 1)
            .s32i(3, 2, 0)
            .raw(&WAITI)
            .raw(&LOOP);
        app
    };
    let handler = pads_high(APP_VECTORS + 0x300, GPIO21);
    let rom = boot_rom();
    let (mapped, unmapped) = (app_code(true), app_code(false));
    let seen = however_split(|| chip(&pro, &rom, &[&mapped, &handler]), 300);
    assert_eq!(
        pad(&seen, 1),
        [(c(35), PadDrive::Low), (c(36), PadDrive::High)]
    );
    let seen = however_split(|| chip(&pro, &rom, &[&unmapped, &handler]), 300);
    assert_eq!(seen.0, []);
}

/// From the captured `CONTROL_0` of a booted chip, 0x1, a store that lets the APP CPU run with
/// no reset since power-on ends the run, as the model cannot know its state. Stores that keep
/// it held do not.
#[test]
fn an_app_cpu_never_reset_does_not_run() {
    let mut program = Program::new(&[SYSTEM]);
    program
        .l32r(2, 0)
        .movi(3, 1)
        .s32i(3, 2, 0)
        .movi(3, 3)
        .s32i(3, 2, 0)
        .movi(3, 2)
        .s32i(3, 2, 0)
        .raw(&LOOP);
    let refused = Refused::Register("SYSTEM", "app_cpu_without_reset");
    let mut chip = chip(&program, &boot_rom(), &[]);
    chip.soc.restore(SYSTEM, 0x1);
    assert_eq!(
        chip.run_until(c(100)),
        Some(Halt::Fault(
            c(6),
            Fault::Refused(refused, pro(program.entry() + 18))
        ))
    );
}

/// The RTC stall freezes the APP CPU's toggles of GPIO4, and its clear resumes them, at the
/// same ticks however the run splits.
#[test]
fn the_rtc_stall_freezes_the_app_cpu() {
    let mut pro = Program::new(&[SYSTEM, APP + 8, RTC_CNTL, 0x21 << 20]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .delay(7, 20)
        .l32r(5, 2)
        .movi(6, 2)
        .s32i(6, 5, 0)
        .l32r(7, 3)
        .s32i(7, 5, 0xbc)
        .delay(8, 40)
        .movi(6, 0)
        .s32i(6, 5, 0xbc)
        .raw(&LOOP);
    let app_code = toggles(APP, 60);
    let rom = boot_rom();
    let seen = however_split(|| chip(&pro, &rom, &[&app_code]), 600);
    let gpio4 = pad(&seen, 0);
    assert_eq!(gpio4.len(), 121, "every toggle runs");
    let gaps: Vec<u64> = gpio4
        .windows(2)
        .map(|w| (w[1].0.0 - w[0].0.0) / XTAL)
        .collect();
    assert_eq!(gaps.iter().filter(|&&gap| gap > 1).count(), 1, "{gaps:?}");
    assert!(gaps.iter().any(|&gap| gap > 100), "{gaps:?}");
}

/// `RTC_CNTL_SW_APPCPU_RST` resets the APP CPU, halted at its spin, and it runs its ROM and
/// its code again. The bit reads 0.
#[test]
fn sw_appcpu_rst_restarts_the_app_cpu_from_its_reset_vector() {
    let mut pro = Program::new(&[SYSTEM, APP + 8, RTC_CNTL]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .delay(7, 30)
        .l32r(5, 2)
        .movi(6, 0x10)
        .s32i(6, 5, 0)
        .raw(&LOOP);
    let app_code = toggles(APP, 1);
    let rom = boot_rom();
    let seen = however_split(|| chip(&pro, &rom, &[&app_code]), 400);
    let drives: Vec<PadDrive> = pad(&seen, 0).into_iter().map(|(_, drive)| drive).collect();
    use PadDrive::{High, Low};
    assert_eq!(drives, [Low, High, Low, High, Low]);
    let mut chip = chip(&pro, &rom, &[&app_code]);
    while chip.run_until(c(400)).is_some() {}
    let options0 = chip.soc.bus().load(Cycle(0), RTC_CNTL, Width::Word);
    assert_eq!(options0, Ok(0));
}

/// The APP CPU's `CCOUNT` after a hold the PRO CPU makes with four stores to `CONTROL_0`, and
/// the PRO CPU's cycles from the second store to the third and from the first to the fourth.
fn ccount_after(stores: [u8; 4]) -> (u32, u32, u32) {
    let mut pro = Program::new(&[SYSTEM, APP + 4, DATA + 0x100]);
    pro.l32r(2, 0).l32r(4, 1).release().delay(7, 10).l32r(9, 2);
    for (k, value) in stores.into_iter().enumerate() {
        pro.movi(3, value)
            .rsr(8, CCOUNT)
            .s32i(3, 2, 0)
            .s32i(8, 9, 4 * k as u8)
            .delay(7, 10);
    }
    pro.raw(&LOOP);
    let mut app = Program::at(APP, &[DATA]);
    app.l32r(2, 0);
    for _ in 0..200 {
        app.rsr(5, CCOUNT).s32i(5, 2, 0);
    }
    app.raw(&LOOP);
    let mut chip = chip(&pro, &boot_rom(), &[&app]);
    assert!(matches!(
        chip.run_until(c(2000)),
        Some(Halt::Fault(
            _,
            Fault::Halted(Pc {
                core: CoreId::App,
                ..
            })
        ))
    ));
    let mut word = |at| chip.soc.bus().load(Cycle(0), at, Width::Word).unwrap();
    let starts = [0, 1, 2, 3].map(|k| word(DATA + 0x100 + 4 * k) + 1);
    (word(DATA), starts[2] - starts[1], starts[3] - starts[0])
}

/// `CCOUNT` counts through a `RUNSTALL` hold with the clock on, and not while `CLKGATE_EN`
/// gates the clock. A gate edge while `RUNSTALL` already holds the APP CPU splits the hold
/// there, so only the gated middle goes uncounted.
#[test]
fn ccount_counts_a_stall_and_not_a_gated_clock() {
    let (stalled, middle, whole) = ccount_after([3, 3, 3, 2]);
    let (gated, ..) = ccount_after([0, 0, 0, 2]);
    assert_eq!(stalled - gated, whole, "a gate of {whole} cycles");
    let (gated_middle, ..) = ccount_after([3, 1, 3, 2]);
    assert_eq!(
        stalled - gated_middle,
        middle,
        "a gate of {middle} cycles under the stall"
    );
    assert!(0 < middle && middle < whole);
}

/// The APP CPU from its reset vector stores to TIMG0's watchdog feed in the step that starts on
/// the cycle the PRO CPU's straddling store to it starts, which lands a cycle later. The APP
/// CPU's store comes second in the order, and the watchdog takes it at the PRO CPU's tick, not
/// before.
#[test]
fn a_peripheral_never_sees_time_go_back() {
    let mut pro = Program::new(&[SYSTEM, TIMG0]);
    pro.l32r(2, 0)
        .movi(3, 2)
        .l32r(5, 1)
        .movi(6, 0)
        .s32i(3, 2, 0)
        .straddling_s32i(6, 5, 0x60)
        .raw(&LOOP);
    let rom = |feeds: bool| {
        let mut rom = Program::at(RESET - 4, &[TIMG0]);
        rom.l32r(2, 0).movi(3, 0).nop().nop();
        if feeds {
            rom.s32i(3, 2, 0x60);
        } else {
            rom.nop();
        }
        rom.raw(&LOOP);
        rom
    };
    let deadline = |feeds| {
        let mut chip = chip(&pro, &rom(feeds), &[]);
        chip.soc
            .watchdogs
            .write_timg(Tick(0), 0, 0x48, 0x0004_c000)
            .unwrap();
        assert_eq!(chip.run_until(c(100)), None);
        chip.soc.watchdogs.deadline()
    };
    assert_eq!(deadline(true), deadline(false));
}

/// The PRO CPU switches the clock from the crystal to 240 MHz with a straddling store, on a
/// cycle the APP CPU polls, while the APP CPU runs, is halted and then woken by an NMI route, or
/// is held by `RUNSTALL` and then released. Both cores then toggle a pad every cycle, GPIO4 the
/// APP CPU and GPIO21 the PRO CPU, and each change comes at a later tick than the one before,
/// however the run splits.
#[test]
fn a_clock_switch_leaves_the_other_core_on_the_axis() {
    #[derive(Clone, Copy, Debug)]
    enum Other {
        Running,
        Halted,
        Held,
    }
    for other in [Other::Running, Other::Halted, Other::Held] {
        let mut app = Program::at(APP, &[APP_VECTORS, GPIO, GPIO4]);
        app.l32r(2, 0).wsr(2, VECBASE);
        if let Other::Halted = other {
            app.raw(&LOOP);
        } else {
            app.l32r(2, 1).l32r(3, 2).s32i(3, 2, 0x24);
            for _ in 0..80 {
                app.s32i(3, 2, 0x08).s32i(3, 2, 0x0c);
            }
            app.raw(&LOOP);
        }
        let nmi = toggles(APP_VECTORS + 0x2c0 - 8, 40);
        let mut pro = Program::new(&[
            SYSTEM,
            app.entry(),
            0x400,
            GPIO,
            GPIO21,
            INTERRUPT_CORE1 + 4 * 81,
        ]);
        pro.l32r(2, 0)
            .l32r(4, 1)
            .movi(6, 0xe)
            .s32i(6, 2, 0x10)
            .l32r(7, 2)
            .release()
            .delay(8, 10);
        if let Other::Held = other {
            pro.movi(3, 3).s32i(3, 2, 0).delay(8, 5);
        }
        pro.straddling_s32i(7, 2, 0x60);
        match other {
            Other::Running => {}
            Other::Halted => {
                pro.movi(6, 1)
                    .s32i(6, 2, 0x38)
                    .l32r(5, 5)
                    .movi(6, 14)
                    .s32i(6, 5, 0);
            }
            Other::Held => {
                pro.movi(3, 2).s32i(3, 2, 0);
            }
        }
        pro.l32r(9, 3).l32r(10, 4).s32i(10, 9, 0x24);
        for _ in 0..40 {
            pro.s32i(10, 9, 0x08).s32i(10, 9, 0x0c);
        }
        pro.raw(&LOOP);
        let seen = however_split(|| chip(&pro, &boot_rom(), &[&app, &nmi]), 100);
        for k in [0, 1] {
            let changes = pad(&seen, k);
            assert!(changes.len() > 40, "{other:?} pad {k}: {changes:?}");
            assert!(
                changes.windows(2).all(|w| w[0].0 < w[1].0),
                "{other:?} pad {k}: {changes:?}"
            );
        }
    }
}

/// The APP CPU switches the clock from the crystal to 240 MHz while the PRO CPU toggles GPIO21
/// with straddling stores, each of which lands a cycle after its step's start. Each delay puts
/// the switch on another cycle of the PRO CPU's toggles, one of them on a tie with a straddling
/// store that lands first. Both cores then toggle a pad every cycle, GPIO4 the APP CPU and
/// GPIO21 the PRO CPU, and each pad's changes come at rising ticks however the run splits.
#[test]
fn a_clock_switch_by_the_app_cpu_keeps_time_moving_on_a_tie() {
    let mut pro = Program::new(&[SYSTEM, APP + 16, GPIO, GPIO21]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .movi(6, 0xe)
        .s32i(6, 2, 0x10)
        .release()
        .l32r(9, 2)
        .l32r(10, 3)
        .s32i(10, 9, 0x24);
    for _ in 0..10 {
        pro.straddling_s32i(10, 9, 0x08)
            .straddling_s32i(10, 9, 0x0c);
    }
    for _ in 0..30 {
        pro.s32i(10, 9, 0x08).s32i(10, 9, 0x0c);
    }
    pro.raw(&LOOP);
    for delay in 0..8 {
        let mut app = Program::at(APP, &[SYSTEM, 0x400, GPIO, GPIO4]);
        app.l32r(2, 0)
            .l32r(5, 1)
            .l32r(6, 2)
            .l32r(7, 3)
            .s32i(7, 6, 0x24);
        for _ in 0..delay {
            app.nop();
        }
        app.s32i(5, 2, 0x60).delay(8, 30);
        for _ in 0..30 {
            app.s32i(7, 6, 0x08).s32i(7, 6, 0x0c);
        }
        app.raw(&LOOP);
        let seen = however_split(|| chip(&pro, &boot_rom(), &[&app]), 150);
        for k in [0, 1] {
            let changes = pad(&seen, k);
            assert!(changes.len() > 40, "delay {delay} pad {k}: {changes:?}");
            assert!(
                changes.windows(2).all(|w| w[0].0 < w[1].0),
                "delay {delay} pad {k}: {changes:?}"
            );
        }
    }
}

/// The PRO CPU's straddling store to a register whose writes are listed starts on the cycle
/// the APP CPU's store to it starts, from the APP CPU's reset vector. The PRO CPU's store stops
/// to be listed, a cycle past its step's start, and lands before the APP CPU's store runs. Each
/// write is listed once, against the core that made it, and the APP CPU's value is kept.
#[test]
fn a_stopped_store_runs_again_before_the_other_cores_step_on_a_tie() {
    let register = RTC_CNTL + 0x74;
    for (first, second) in [(1, 2), (1, 1)] {
        let mut program = Program::new(&[SYSTEM, RTC_CNTL]);
        program
            .l32r(2, 0)
            .movi(3, 2)
            .l32r(5, 1)
            .movi(6, first)
            .s32i(3, 2, 0)
            .straddling_s32i(6, 5, 0x74)
            .raw(&LOOP);
        let pro_store = program.pc() - 6;
        let mut rom = Program::at(RESET - 4, &[RTC_CNTL]);
        rom.l32r(2, 0).movi(3, second).nop().nop();
        let app_store = rom.pc();
        rom.s32i(3, 2, 0x74).raw(&LOOP);
        let mut chip = chip(&program, &rom, &[]);
        chip.soc.restore(register, 0);
        while chip.run_until(c(100)).is_some() {}
        let listed: Vec<(u32, Pc)> = chip
            .unmodeled()
            .map(|(address, touch)| (address, touch.pc))
            .collect();
        let mut expected = vec![(register, pro(pro_store))];
        if second != first {
            expected.push((register, app(app_store)));
        }
        assert_eq!(listed, expected, "{first} then {second}");
        let ticks: Vec<Tick> = chip.unmodeled().map(|(_, touch)| touch.at).collect();
        assert!(
            ticks.windows(2).all(|w| w[0] == w[1]),
            "one tick: {ticks:?}"
        );
        let kept = chip.soc.bus().load(Cycle(0), register, Width::Word);
        assert_eq!(kept, Ok(u32::from(second)));
    }
}

/// The PRO CPU's straddling unaligned store starts on the cycle the APP CPU's store to
/// `RTC_CNTL_SW_CPU_STALL_REG` starts. Its first half clears that register's high half, and its
/// second half stops at a word the model does not keep, a cycle past its step's start. The
/// store runs again before the APP CPU's store, so the APP CPU's value is the one kept.
#[test]
fn a_store_stopped_at_an_unmodeled_word_runs_again_before_the_other_cores_step_on_a_tie() {
    let stall = RTC_CNTL + 0xbc;
    let mut program = Program::new(&[SYSTEM, RTC_CNTL + 0xbe]);
    program
        .l32r(2, 0)
        .movi(3, 2)
        .l32r(5, 1)
        .movi(6, 0)
        .s32i(3, 2, 0)
        .straddling_s32i(6, 5, 0)
        .raw(&LOOP);
    let pro_store = program.pc() - 6;
    let mut rom = Program::at(RESET - 8, &[stall, 0xf << 16]);
    rom.l32r(2, 0)
        .l32r(3, 1)
        .nop()
        .nop()
        .s32i(3, 2, 0)
        .raw(&LOOP);
    let mut chip = chip(&program, &rom, &[]);
    while chip.run_until(c(100)).is_some() {}
    let listed: Vec<(u32, Pc)> = chip
        .unmodeled()
        .map(|(address, touch)| (address, touch.pc))
        .collect();
    assert_eq!(listed, [(RTC_CNTL + 0xc0, pro(pro_store))]);
    assert_eq!(
        chip.soc.bus().load(Cycle(0), stall, Width::Word),
        Ok(0xf << 16)
    );
}

/// The PRO CPU halts at its spin while the APP CPU runs, and the APP CPU then stalls itself, so
/// no core may run. The run ends with `chip.halted` at the PRO CPU's spin, on the cycle the APP
/// CPU stopped at, or, while a watchdog runs, with that watchdog at its deadline. With no core
/// to run, the chip's tick is the limit, so only the halts match however the run splits.
#[test]
fn a_run_with_no_core_that_may_run_halts_or_waits_for_its_watchdog() {
    let mut program = Program::new(&[SYSTEM, APP + 4]);
    program.l32r(2, 0).l32r(4, 1).release();
    let spin = program.pc();
    program.raw(&LOOP);
    let mut app = Program::at(APP, &[SYSTEM]);
    app.l32r(4, 0)
        .delay(6, 20)
        .movi(5, 3)
        .s32i(5, 4, 0)
        .raw(&LOOP);
    let rom = boot_rom();
    let halts = |step| seen(chip(&program, &rom, &[&app]), 400, step).0;
    let whole = halts(400);
    for step in [1, 2, 3, 7] {
        assert_eq!(halts(step), whole, "steps of {step} cycles");
    }
    assert_eq!(
        whole,
        [(
            Halt::Fault(c(111), Fault::Halted(pro(spin))),
            [PadDrive::Off, PadDrive::Off]
        )]
    );
    let mut chip = chip(&program, &rom, &[&app]);
    chip.soc
        .watchdogs
        .write_timg(Tick(0), 0, 0x48, 0x0004_c000)
        .unwrap();
    let expiry = chip.soc.watchdogs.deadline().expect("TIMG0 runs");
    let due = chip.soc.clock.round_up(expiry.at);
    assert!(matches!(
        chip.run_until(Tick(u64::MAX - 1)),
        Some(Halt::Fault(at, Fault::Watchdog { watchdog, stage: 0, .. }))
            if at == due && watchdog == expiry.watchdog
    ));
}

/// A part with no ROM that releases the APP CPU ends the run with an unmapped fetch at the
/// reset vector, on the APP CPU.
#[test]
fn an_app_cpu_released_with_no_rom_fetches_nothing() {
    let mut pro = Program::new(&[SYSTEM, APP]);
    pro.l32r(2, 0).l32r(4, 1).release().raw(&LOOP);
    let mut chip = pro.chip();
    assert_eq!(
        chip.run_until(c(100)),
        Some(Halt::Fault(c(4), Fault::UnmappedFetch(app(RESET))))
    );
}

/// From the captured `CONTROL_0`, 0x1, the PRO CPU turns the RTC stall on, sets `CLKGATE_EN`
/// and clears `RUNSTALL`. The APP CPU stays held, and the clear of the RTC stall that would let
/// it run ends the run.
#[test]
fn a_clear_of_the_rtc_stall_that_lets_an_app_cpu_never_reset_run_ends_the_run() {
    let mut program = Program::new(&[SYSTEM, RTC_CNTL, 0x21 << 20]);
    program
        .l32r(2, 0)
        .l32r(5, 1)
        .movi(6, 2)
        .s32i(6, 5, 0)
        .l32r(7, 2)
        .s32i(7, 5, 0xbc)
        .movi(3, 3)
        .s32i(3, 2, 0)
        .movi(3, 2)
        .s32i(3, 2, 0)
        .movi(6, 0);
    let clear = program.pc();
    program.s32i(6, 5, 0xbc).raw(&LOOP);
    let refused = Refused::Register("SYSTEM", "app_cpu_without_reset");
    let mut chip = chip(&program, &boot_rom(), &[]);
    chip.soc.restore(SYSTEM, 0x1);
    assert!(matches!(
        chip.run_until(c(100)),
        Some(Halt::Fault(_, Fault::Refused(why, pc))) if why == refused && pc == pro(clear)
    ));
}

/// The APP CPU makes the four stores of ESP-IDF's `esp_cpu_stall(0)`, a clear and then a set of
/// the PRO CPU's `C0` and then of its `C1`. The last one completes the pattern and ends the
/// run, as the model does not stall the PRO CPU.
#[test]
fn a_store_that_would_stall_the_pro_cpu_ends_the_run() {
    let mut pro = Program::new(&[SYSTEM, APP + 8]);
    pro.l32r(2, 0).l32r(4, 1).release().raw(&LOOP);
    let mut app_code = Program::at(APP, &[RTC_CNTL, 0x21 << 26]);
    app_code
        .l32r(2, 0)
        .l32r(3, 1)
        .movi(4, 2 << 2)
        .movi(5, 0)
        .s32i(5, 2, 0)
        .s32i(4, 2, 0)
        .s32i(5, 2, 0xbc);
    let completes = app_code.pc();
    app_code.s32i(3, 2, 0xbc).raw(&LOOP);
    let refused = Refused::Register("RTC_CNTL", "pro_cpu_stall");
    let mut chip = chip(&pro, &boot_rom(), &[&app_code]);
    assert!(matches!(
        chip.run_until(c(200)),
        Some(Halt::Fault(_, Fault::Refused(why, pc))) if why == refused && pc == app(completes)
    ));
}

/// The APP CPU's unaligned store turns its own RTC stall on with its first half and stops at a
/// word the model does not keep with its second. The stall holds the APP CPU, so the PRO CPU
/// runs on through a delay and sets GPIO4, and the touch is listed against the store.
#[test]
fn a_stopped_store_that_holds_its_own_core_lets_the_other_core_run_on() {
    let mut pro = Program::new(&[SYSTEM, APP + 12, GPIO, GPIO4]);
    pro.l32r(2, 0)
        .l32r(4, 1)
        .release()
        .delay(5, 40)
        .l32r(2, 2)
        .l32r(3, 3)
        .s32i(3, 2, 0x24)
        .s32i(3, 2, 0x08)
        .raw(&LOOP);
    let mut app_code = Program::at(APP, &[RTC_CNTL, RTC_CNTL + 0xbe, 0x21 << 4]);
    app_code
        .l32r(2, 0)
        .l32r(5, 1)
        .l32r(6, 2)
        .movi(4, 2)
        .s32i(4, 2, 0);
    let store = app_code.pc();
    app_code.s32i(6, 5, 0).raw(&LOOP);
    let mut chip = chip(&pro, &boot_rom(), &[&app_code]);
    while chip.run_until(c(400)).is_some() {}
    assert_eq!(chip.drive(4), PadDrive::High, "the PRO CPU ran on");
    let touched: Vec<(u32, Pc, bool)> = chip
        .unmodeled()
        .map(|(address, touch)| (address, touch.pc, touch.write))
        .collect();
    assert_eq!(touched, [(RTC_CNTL + 0xc0, app(store), true)]);
}

/// A load by the APP CPU of a register the model does not keep is listed with the APP CPU's PC.
#[test]
fn an_unmodeled_register_the_app_cpu_reads_is_listed_against_it() {
    let mut pro = Program::new(&[SYSTEM, APP + 4]);
    pro.l32r(2, 0).l32r(4, 1).release().raw(&LOOP);
    let mut app_code = Program::at(APP, &[GPIO]);
    app_code.l32r(2, 0);
    let load = app_code.pc();
    app_code.l32i(3, 2, 0x74).raw(&LOOP);
    let mut chip = chip(&pro, &boot_rom(), &[&app_code]);
    while chip.run_until(c(200)).is_some() {}
    let touched: Vec<(u32, Pc, bool)> = chip
        .unmodeled()
        .map(|(address, touch)| (address, touch.pc, touch.write))
        .collect();
    assert_eq!(touched, [(GPIO + 0x74, app(load), false)]);
}

/// From `at`, enables GPIO21, holds timer group 0's stage 0 to 0, starts the watchdog with that
/// stage resetting the system, and sets GPIO21 in the next instruction.
fn arm_then_set(at: u32) -> Program {
    let mut program = Program::at(at, &[GPIO, GPIO21, TIMG0, EN | 3 << 29]);
    program
        .l32r(2, 0)
        .l32r(3, 1)
        .s32i(3, 2, 0x24)
        .l32r(4, 2)
        .movi(5, 0)
        .s32i(5, 4, 0x50)
        .l32r(5, 3)
        .s32i(5, 4, 0x48)
        .s32i(3, 2, 0x08)
        .raw(&LOOP);
    program
}

/// The watchdog start of [`arm_then_set`], on either core while the other runs, ends the run
/// at the next cycle, before the next instruction sets GPIO21, however the run splits. On the
/// PRO CPU, the APP CPU runs a delay loop from each of four offsets, so on some the APP CPU's
/// steps tie with the start and on others they skip its cycle.
#[test]
fn a_watchdog_stage_due_just_after_its_start_comes_before_either_cores_next_instruction() {
    let rom = boot_rom();
    let mut cases = Vec::new();
    for offset in 0..4 {
        let mut pro = Program::new(&[SYSTEM, APP]);
        pro.l32r(2, 0).l32r(4, 1).release().j(BASE + 0x800);
        let mut delay = Program::at(APP, &[]);
        for _ in 0..offset {
            delay.nop();
        }
        delay.delay(5, 60).raw(&LOOP);
        let more = vec![arm_then_set(BASE + 0x800 - 16), delay];
        cases.push((format!("PRO, offset {offset}"), pro, more));
    }
    let mut idle = Program::new(&[SYSTEM, APP + 16]);
    idle.l32r(2, 0).l32r(4, 1).release().delay(6, 30).raw(&LOOP);
    cases.push(("APP".to_string(), idle, vec![arm_then_set(APP)]));
    for (core, pro, more) in cases {
        let more: Vec<&Program> = more.iter().collect();
        let seen = however_split(|| chip(&pro, &rom, &more), 300);
        let [(enabled, PadDrive::Low)] = pad(&seen, 1)[..] else {
            panic!("{core}: GPIO21 is enabled and never set: {seen:?}");
        };
        assert_eq!(
            seen.0.last().map(|&(halt, _)| halt),
            Some(Halt::Fault(Tick(enabled.0 + 6 * XTAL), TIMG0_FLASHBOOT)),
            "{core}"
        );
    }
}

/// The PRO CPU releases the APP CPU into `nop`s of one cycle each when `both`, and runs three
/// `nop`s in its place otherwise. It then sets `VECBASE`, lets level-1 interrupt 1 in, holds
/// timer group 0's stage 0 to `hold`, starts the watchdog with that stage interrupting, and
/// makes a straddling store to an unmodeled GPIO word. Returns the chip and that store's PC.
/// Timer group 0's watchdog source reaches interrupt 1, whose handler raises GPIO21.
fn armed_then_listed(hold: u8, both: bool) -> (Esp32s3, u32) {
    let mut pro = Program::new(&[
        SYSTEM,
        APP,
        TIMG0,
        EN | 1 << 29 | 3 << 27,
        PRO_VECTORS,
        GPIO,
    ]);
    pro.l32r(2, 0).l32r(4, 1);
    if both {
        pro.release();
    } else {
        pro.nop().nop().nop();
    }
    pro.l32r(5, 4)
        .wsr(5, VECBASE)
        .movi(7, 2)
        .wsr(7, INTENABLE)
        .movi(7, 0)
        .wsr(7, PS)
        .l32r(8, 2)
        .movi(9, hold)
        .s32i(9, 8, 0x50)
        .l32r(9, 3)
        .s32i(9, 8, 0x48)
        .l32r(10, 5)
        .straddling_s32i(9, 10, 0x74)
        .raw(&LOOP);
    let store = pro.pc() - 6;
    let mut app_code = Program::at(APP, &[]);
    for _ in 0..200 {
        app_code.nop();
    }
    app_code.raw(&LOOP);
    let handler = pads_high(PRO_VECTORS + 0x300, GPIO21);
    let mut chip = chip(&pro, &boot_rom(), &[&app_code, &handler]);
    chip.soc.restore(TIMG0 + 0x70, 1 << 2);
    chip.soc.restore(INTERRUPT_CORE0 + 4 * 52, 1);
    (chip, store)
}

/// The chip of [`armed_then_listed`] on two cores and on one, for holds around 6. With 6 stage 0
/// runs out on the cycle the store's refetch charged it to, while the APP CPU is a cycle back.
/// Each hold takes the interrupt at the same PC and lists the same store on two cores as on
/// one, however the run splits. With 6 the stage runs out before the store runs again, so
/// `EPC1` holds the store.
#[test]
fn a_watchdog_stage_due_on_a_stopped_stores_charged_cycle_runs_out_before_its_rerun() {
    let run = |hold, both| {
        let halts = however_split(|| armed_then_listed(hold, both).0, 300).0;
        let (mut chip, store) = armed_then_listed(hold, both);
        while chip.run_until(c(300)).is_some() {}
        let epc1 = chip.cores.core(CoreId::Pro).special(Sr::Epc1);
        let listed: Vec<(u32, Pc)> = chip
            .unmodeled()
            .map(|(address, touch)| (address, touch.pc))
            .collect();
        (halts, epc1, listed, store)
    };
    for hold in 4..9 {
        assert_eq!(run(hold, true), run(hold, false), "hold {hold}");
    }
    let (_, epc1, listed, store) = run(6, true);
    assert_eq!(epc1, store);
    assert_eq!(listed, [(GPIO + 0x74, pro(store))]);
}

mod waiting;
