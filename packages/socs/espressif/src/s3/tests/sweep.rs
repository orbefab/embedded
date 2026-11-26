//! What the chip promises about a loop's passes, as `Bus::sweep` asks: each load of each pass
//! lies in the range it gave, and the passes leave the chip as the last pass alone leaves it.
//! A program that polls SYSTIMER as `millis()` does runs alike whether the core skips its
//! passes or steps each op.

use orbe_xtensa::{Config, PassAccess, Swept};

use super::app::Steer;
use super::*;

/// Data SRAM, which the data bus reaches.
const DRAM: u32 = 0x3fc9_0000;
const OP0: u32 = SYSTIMER + 0x04;
const HI0: u32 = SYSTIMER + 0x40;
const LO0: u32 = SYSTIMER + 0x44;
const UPDATE: u32 = 1 << 30;
const VALUE_VALID: u32 = 1 << 29;

fn load(addr: u32, offset: u64) -> PassAccess {
    PassAccess {
        addr,
        width: Width::Word,
        store: None,
        offset,
    }
}

fn store(addr: u32, value: u32, offset: u64) -> PassAccess {
    PassAccess {
        addr,
        width: Width::Word,
        store: Some(value),
        offset,
    }
}

/// IDF's `systimer_hal_get_counter_value` for unit 0, as `millis()` reaches it: read `OP`, set
/// `UPDATE`, poll `VALUE_VALID`, then `LO`, `HI` and `LO` again, then a word of RAM.
fn millis() -> Vec<PassAccess> {
    vec![
        load(OP0, 0),
        store(OP0, VALUE_VALID | UPDATE, 3),
        load(OP0, 5),
        load(LO0, 9),
        load(HI0, 12),
        load(LO0, 15),
        load(DRAM, 40),
    ]
}

/// A chip with a word in RAM whose unit 0 has latched once, as it has after the first
/// `millis()`.
fn idle_chip() -> Esp32s3 {
    let mut program = Program::new(&[]);
    program.raw(&LOOP);
    let mut chip = program.chip();
    let mut bus = chip.soc.bus();
    bus.store(Cycle(0), DRAM, Width::Word, 0x1234_5678).unwrap();
    bus.store(Cycle(100), OP0, Width::Word, UPDATE).unwrap();
    chip
}

/// Makes `accesses` on `soc` as a pass that starts at `start` would, and returns what each
/// access read or wrote.
fn pass(soc: &mut Soc, accesses: &[PassAccess], start: u64) -> Vec<u32> {
    let mut bus = soc.bus();
    accesses
        .iter()
        .map(|a| {
            let at = Cycle(start + a.offset);
            match a.store {
                Some(value) => {
                    bus.store(at, a.addr, a.width, value).unwrap();
                    value
                }
                None => bus.load(at, a.addr, a.width).unwrap(),
            }
        })
        .collect()
}

/// Every pass made, against the ranges, and the last pass alone, against every pass.
fn sweep_holds(chip: &Esp32s3, accesses: &[PassAccess], start: u64, period: u64, passes: u64) {
    let mut out = vec![Swept::default(); accesses.len()];
    let at = format!("start {start} period {period} passes {passes}");
    assert!(
        chip.soc
            .sweep(accesses, Cycle(start), period, passes, &mut out),
        "{at}"
    );
    let mut every = chip.soc.clone();
    for k in 0..passes {
        let read = pass(&mut every, accesses, start + k * period);
        for (i, a) in accesses.iter().enumerate() {
            if a.store.is_some() {
                continue;
            }
            let s = out[i];
            assert!(
                (s.lo..=s.hi).contains(&read[i]),
                "load {i} of pass {k} read {:#x}, out of {s:?}, {at}",
                read[i]
            );
            if let Some(j) = s.same_as {
                assert_eq!(read[i], read[usize::from(j)], "load {i} as {j}, {at}");
            }
        }
    }
    let mut last = chip.soc.clone();
    pass(&mut last, accesses, start + (passes - 1) * period);
    assert_eq!(
        format!("{every:?}"),
        format!("{last:?}"),
        "every pass and the last left the chip apart, {at}"
    );
}

#[test]
fn a_sweep_holds_each_load_and_leaves_what_the_last_pass_leaves() {
    let chip = idle_chip();
    for (start, period, passes) in [
        (1_000, 231, 1),
        (1_000, 231, 2),
        (1_000, 231, 700),
        (5_003, 97, 4_000),
        (20_000, 1_000, 3),
    ] {
        sweep_holds(&chip, &millis(), start, period, passes);
    }
    let out = {
        let mut out = vec![Swept::default(); 7];
        assert!(chip.soc.sweep(&millis(), Cycle(1_000), 231, 700, &mut out));
        out
    };
    assert_eq!((out[2].lo, out[2].hi), (VALUE_VALID, VALUE_VALID));
    assert_eq!(out[5].same_as, Some(3), "LO again since the latch");
    assert!(out[3].hi > out[3].lo, "the count moves over 700 passes");
    assert_eq!((out[6].lo, out[6].hi), (0x1234_5678, 0x1234_5678));
}

/// A load before the pass's latch reads what the unit holds in the first pass, and what the
/// pass before latched in each later one.
#[test]
fn a_load_before_the_latch_reads_the_latch_of_the_pass_before() {
    let chip = idle_chip();
    let early = vec![
        load(LO0, 0),
        load(HI0, 2),
        load(OP0, 4),
        store(OP0, UPDATE, 6),
        load(LO0, 9),
    ];
    for passes in [1, 2, 50, 900] {
        sweep_holds(&chip, &early, 2_000, 150, passes);
    }
    // Before any latch, `VALUE_VALID` would read clear in the first pass and set after it.
    let mut program = Program::new(&[]);
    program.raw(&LOOP);
    let fresh = program.chip();
    let mut out = vec![Swept::default(); early.len()];
    assert!(!fresh.soc.sweep(&early, Cycle(2_000), 150, 3, &mut out));
    assert!(fresh.soc.sweep(&early, Cycle(2_000), 150, 1, &mut out));
}

/// The low word turns over inside the passes: `HI` moves on, and `LO` may read anything.
#[test]
fn a_sweep_across_the_low_word_carry() {
    let mut chip = idle_chip();
    // Unit 0 counts on from just below a carry of its low word.
    let below = (1u64 << 32) - 500;
    let mut bus = chip.soc.bus();
    bus.store(
        Cycle(10),
        SYSTIMER + 0x0c,
        Width::Word,
        (below >> 32) as u32,
    )
    .unwrap();
    bus.store(Cycle(11), SYSTIMER + 0x10, Width::Word, below as u32)
        .unwrap();
    bus.store(Cycle(12), SYSTIMER + 0x5c, Width::Word, 1)
        .unwrap();
    for passes in [2, 10, 100, 2_000] {
        sweep_holds(&chip, &millis(), 1_000, 231, passes);
    }
    let mut out = vec![Swept::default(); 7];
    assert!(
        chip.soc
            .sweep(&millis(), Cycle(1_000), 231, 2_000, &mut out)
    );
    assert_eq!((out[3].lo, out[3].hi), (0, u32::MAX));
    assert_eq!((out[4].lo, out[4].hi), (0, 1));
}

#[test]
fn a_sweep_refuses_what_it_cannot_promise() {
    let chip = idle_chip();
    let mut out = vec![Swept::default(); 8];
    let refuses = |soc: &Soc, accesses: &[PassAccess], out: &mut [Swept]| {
        !soc.sweep(accesses, Cycle(1_000), 231, 10, &mut out[..accesses.len()])
    };
    // A store to RAM, to another SYSTIMER word, a byte latch, or an `OP` store that latches
    // nothing.
    for other in [
        store(DRAM, 1, 20),
        store(SYSTIMER, 0, 20),
        store(SYSTIMER + 0x64, 0, 20),
        store(OP0, VALUE_VALID, 20),
        PassAccess {
            width: Width::Byte,
            ..store(OP0 + 3, 0x40, 20)
        },
    ] {
        let mut accesses = millis();
        accesses.push(other);
        assert!(refuses(&chip.soc, &accesses, &mut out), "{other:?}");
    }
    // A load of a peripheral other than SYSTIMER, or of a SYSTIMER word it does not answer.
    for other in [load(GPIO + 0x3c, 20), load(SYSTIMER + 0x68, 20)] {
        let mut accesses = millis();
        accesses.push(other);
        assert!(refuses(&chip.soc, &accesses, &mut out), "{other:?}");
    }
    assert!(!refuses(&chip.soc, &millis(), &mut out));

    // A steer for the scheduler.
    let mut steered = chip.soc.clone();
    steered.steer = Some(Steer::Wake(Cycle(500)));
    assert!(refuses(&steered, &millis(), &mut out));

    // A DMA transfer that reads SRAM.
    let mut dma = chip.soc.clone();
    dma.hold_in_flight(0..64);
    assert!(refuses(&dma, &millis(), &mut out));

    // An access stamped past the first pass's start, as the other core's can be.
    let mut later = chip.soc.clone();
    later.load_word(Cycle(1_500), LO0).unwrap();
    assert!(refuses(&later, &millis(), &mut out));
    assert!(later.sweep(&millis(), Cycle(1_500), 231, 10, &mut out[..7]));

    // LEDC output waiting, which a SYSTIMER store rearms the run for.
    let mut program = Program::new(&[]);
    program.raw(&LOOP);
    let mut ledc = program.chip();
    let latch = (OP0, UPDATE);
    for &(addr, value) in &[vec![latch], ledc_on(3), duty(2).to_vec()].concat() {
        ledc.soc
            .bus()
            .store(Cycle(0), addr, Width::Word, value)
            .unwrap();
    }
    ledc.soc.rearmed = false;
    ledc.soc.changed = None;
    ledc.soc.ledc.settle(c(10), ledc.soc.clock.source.apb());
    assert!(ledc.soc.ledc.has_output());
    assert!(refuses(&ledc.soc, &millis(), &mut out));
}

/// The PRO CPU's bus, counting the op starts its core marks. A kept one passes the code
/// version and the sweep through, so its core runs straight runs and may skip. One that is not
/// keeps no version, so its core steps each op and skips nothing.
struct Watched<'a> {
    bus: soc::CoreBus<'a>,
    kept: bool,
    starts: u64,
}

impl Bus for Watched<'_> {
    fn instruction_bus(&self, addr: u32) -> bool {
        self.bus.instruction_bus(addr)
    }
    fn fetch(&mut self, now: Cycle, addr: u32) -> Result<u32, Exit> {
        self.bus.fetch(now, addr)
    }
    fn load(&mut self, now: Cycle, addr: u32, width: Width) -> Result<u32, Exit> {
        self.bus.load(now, addr, width)
    }
    fn store(&mut self, now: Cycle, addr: u32, width: Width, value: u32) -> Result<(), Exit> {
        self.bus.store(now, addr, width, value)
    }
    fn interrupts(&mut self, now: Cycle) -> u32 {
        self.starts += 1;
        self.bus.interrupts(now)
    }
    fn begin_step(&mut self, now: Cycle) {
        self.starts += 1;
        self.bus.begin_step(now);
    }
    fn access_moves_lines(&self, addr: u32) -> bool {
        self.bus.access_moves_lines(addr)
    }
    fn load_moves_lines(&self, addr: u32) -> bool {
        self.bus.load_moves_lines(addr)
    }
    fn next_event(&mut self, now: Cycle) -> Option<Cycle> {
        self.bus.next_event(now)
    }
    fn yields(&self) -> bool {
        self.bus.yields()
    }
    fn code_version(&self) -> Option<u64> {
        self.kept.then(|| self.bus.code_version()).flatten()
    }
    fn fetch_held(&self) -> bool {
        self.bus.fetch_held()
    }
    fn sweep(
        &self,
        accesses: &[PassAccess],
        start: Cycle,
        period: u64,
        passes: u64,
        out: &mut [Swept],
    ) -> bool {
        self.kept && self.bus.sweep(accesses, start, period, passes, out)
    }
}

/// `loop()` polling the count as `millis()` does: `OP` read and written back with `UPDATE`,
/// `VALUE_VALID` polled, `LO` and `HI` read, and while the count shifted down by 4 is less than
/// `a5` past the word at `DRAM`, back. Otherwise that word moves on by `a5`, and back.
fn poller() -> Program {
    let mut program = Program::new(&[SYSTIMER, UPDATE, DRAM]);
    program.l32r(2, 0).l32r(3, 1).l32r(4, 2).movi(5, 100);
    let top = program.pc();
    program
        .rri8(2, 6, 2, 0x04)
        .raw(&[0x30, 0x66, 0x20]) // or a6, a6, a3
        .rri8(6, 6, 2, 0x04)
        .rri8(2, 7, 2, 0x04)
        .rri8(2, 8, 2, 0x44)
        .rri8(2, 9, 2, 0x40)
        .raw(&[0x00, 0x44, 0x40]) // ssai 4
        .raw(&[0x80, 0x89, 0x81]) // src a8, a9, a8
        .rri8(2, 10, 4, 0)
        .raw(&[0xa0, 0xb8, 0xc0]); // sub a11, a8, a10
    let at = program.pc();
    let back = (top as i32 - at as i32 - 4) as u8;
    program
        .raw(&[0x57, 0x3b, back]) // bltu a11, a5, top
        .raw(&[0x50, 0xaa, 0x80]) // add a10, a10, a5
        .rri8(6, 10, 4, 0);
    let at = program.pc();
    let offset = (top as i32 - at as i32 - 4) as u32 & 0x3_ffff;
    let j = offset << 6 | 6;
    program.raw(&[j as u8, (j >> 8) as u8, (j >> 16) as u8]);
    program
}

/// The poller on the chip, run on its own bus, which skips the passes between moves of the
/// word, and stepped, which does not: the core, SYSTIMER and the word end alike at each limit,
/// and the skipping run reads far fewer of the cycles op by op.
#[test]
fn a_millis_poll_runs_alike_skipped_and_stepped() {
    let program = poller();
    let listing: Vec<String> = {
        let code = &program.image()[12..];
        let mut at = 0;
        let mut out = Vec::new();
        while at < code.len() {
            let decoded = orbe_xtensa::Esp32s3::decode(&code[at..], program.entry() + at as u32)
                .expect("an op");
            let orbe_xtensa::Decoded::Insn(insn) = &decoded else {
                panic!("{decoded:?}");
            };
            at += usize::from(insn.len);
            out.push(orbe_xtensa::Esp32s3::render(&decoded));
        }
        out
    };
    assert_eq!(
        listing[4..],
        [
            "l32i\ta6, a2, 4",
            "or\ta6, a6, a3",
            "s32i\ta6, a2, 4",
            "l32i\ta7, a2, 4",
            "l32i\ta8, a2, 68",
            "l32i\ta9, a2, 64",
            "ssai\t4",
            "src\ta8, a9, a8",
            "l32i\ta10, a4, 0",
            "sub\ta11, a8, a10",
            "bltu\ta11, a5, 0x40370018",
            "add\ta10, a10, a5",
            "s32i\ta10, a4, 0",
            "j\t0x40370018",
        ]
    );
    for limit in [30_000, 200_000, 200_017, 400_000] {
        let run = |chip: &mut Esp32s3, kept: bool| {
            let mut core = chip.cores.core(CoreId::Pro).clone();
            let mut bus = Watched {
                bus: chip.soc.bus(),
                kept,
                starts: 0,
            };
            assert_eq!(
                core.run_until(&mut bus, Cycle(limit)),
                orbe_xtensa::Stop::Limit
            );
            (core, bus.starts)
        };
        let mut skipped = program.chip();
        let (core, starts) = run(&mut skipped, true);
        let mut stepped = program.chip();
        let (plain, steps) = run(&mut stepped, false);
        assert_eq!(core, plain, "limit {limit}");
        if limit >= 200_000 && cfg!(not(feature = "pass-skip-check")) {
            assert!(starts * 3 < steps, "{starts} of {steps} at {limit}");
        }
        assert_eq!(
            format!("{:?}", skipped.soc.systimer),
            format!("{:?}", stepped.soc.systimer),
            "limit {limit}"
        );
        let word = |soc: &mut Soc| soc.load_word(Cycle(limit + 1_000), DRAM).unwrap();
        let moved = word(&mut skipped.soc);
        assert_eq!(moved, word(&mut stepped.soc), "limit {limit}");
        assert!(moved >= 100 * (limit as u32 / 40_000), "{moved} at {limit}");
    }
}
