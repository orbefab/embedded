use crate::world::World;
use orbe_model::{Drive, Pulses, Stop, Time};

use crate::active::native::Hosted;
use crate::diagnostic::ProtocolRule;
use crate::leds::{self, Chain};
use crate::link::{Answers, LinkRow, Record, RecordKind};
use crate::netlist::NetId;

/// The sketch's bits, high then low.
const ZERO: (u64, u64) = (312_500, 937_500);
const ONE: (u64, u64) = (625_000, 625_000);
const RESET_LOW: u64 = 300_000_000;
const START: u64 = 1_000_000_000;
/// RES is a low longer than this. Exactly this long is no reset.
const RES: u64 = 280_000_000;

/// The SK6812MINI-HS's draw: 12 mA a channel at 255, 1 mA idle, held above 3.7 V.
fn draw() -> super::PixelDraw {
    let value = |si| crate::netlist::Value {
        si: crate::finite::Finite::new(si).unwrap(),
    };
    super::PixelDraw {
        channel_current: value(0.012),
        idle_current: value(0.001),
        holds_above: value(3.7),
    }
}

/// The amperes [`draw`] takes for `count` LEDs whose latched bytes add to `lit`.
fn amps(count: u16, lit: u32) -> f64 {
    f64::from(count) * 0.001 + 0.012 * f64::from(lit) / 255.0
}

/// A chain's core, as the engine builds it for a part of `count` LEDs.
struct Core {
    hosted: Box<dyn Hosted>,
}

struct Latch {
    at: Time,
    pixels: Vec<[u8; 3]>,
}

impl Clone for Core {
    fn clone(&self) -> Self {
        Core {
            hosted: self.hosted.boxed(),
        }
    }
}

impl Core {
    fn new(count: u16) -> Core {
        Core::with_dout(count, 0)
    }

    /// A chain that drives `DOUT` `delay` picoseconds after `DIN`, or has none at 0.
    fn with_dout(count: u16, delay: u64) -> Core {
        let dout = (delay > 0).then_some(NetId::GROUND);
        let mut hosted = super::core(count, delay, dout, None, None);
        hosted.keep_shown();
        Core { hosted }
    }

    /// A chain of `count` that draws [`draw`] and has no `DOUT`.
    fn drawing(count: u16) -> Core {
        let mut hosted = super::core(count, 0, None, Some(&draw()), None);
        hosted.keep_shown();
        Core { hosted }
    }

    fn run_until(&mut self, limit: Time) -> Option<Stop> {
        self.hosted.run_until(&World::default(), limit)
    }

    fn input(&mut self, at: Time, levels: &[bool]) {
        self.hosted.input(&World::default(), at, levels);
    }

    fn land(&mut self, start: Time, idle: bool, pulses: &[(bool, u64)], end: Time) {
        let row = record(start.as_ps(), idle, pulses);
        debug_assert_eq!(row.record.end, end);
        self.hosted
            .take(&World::default(), end, &[row], &mut Answers::default());
    }

    fn observed(&self) -> Chain {
        Chain::shown(self.hosted.shows(), self.hosted.view())
    }

    fn latches(&self) -> Vec<Latch> {
        self.hosted
            .shown()
            .iter()
            .map(|shown| Latch {
                at: shown.at,
                pixels: leds::pixels(&shown.bytes),
            })
            .collect()
    }

    fn broke(&self) -> Option<(ProtocolRule, Time)> {
        let mut found = self.hosted.broke();
        assert!(found.len() <= 1, "a chain warns once");
        found.pop()
    }
}

fn record(start: u64, idle: bool, pulses: &[(bool, u64)]) -> LinkRow {
    let end = start + pulses.iter().map(|&(_, ps)| ps).sum::<u64>();
    LinkRow {
        sender: 0,
        record: Record {
            seq: 0,
            start: Time::from_ps(start),
            end: Time::from_ps(end),
            pin: "DIN",
            kind: RecordKind::Pulses {
                pulses: Pulses {
                    idle,
                    pulses: pulses.into(),
                },
                net: NetId::GROUND,
            },
        },
    }
}

fn bits(bytes: &[u8]) -> Vec<bool> {
    bytes
        .iter()
        .flat_map(|byte| (0..8).rev().map(move |k| byte & (1 << k) != 0))
        .collect()
}

/// Each bit as a high and a low, the last low lengthened by the reset.
fn train(bytes: &[u8], zero: (u64, u64), one: (u64, u64)) -> Vec<(bool, u64)> {
    let bits = bits(bytes);
    let mut out = Vec::with_capacity(bits.len() * 2);
    for (i, bit) in bits.iter().enumerate() {
        let (high, mut low) = if *bit { one } else { zero };
        if i + 1 == bits.len() {
            low += RESET_LOW;
        }
        out.push((true, high));
        out.push((false, low));
    }
    out
}

fn end(start: u64, pulses: &[(bool, u64)]) -> u64 {
    start + pulses.iter().map(|&(_, ps)| ps).sum::<u64>()
}

/// One past the reset of the train's last low.
fn reset_after(start: u64, pulses: &[(bool, u64)]) -> u64 {
    end(start, pulses) - pulses.last().unwrap().1 + RES + 1
}

/// Every stop up to `t` lands, as the seam lands them before anything else at `t`.
fn settle(core: &mut Core, t: u64) {
    while core.run_until(Time::from_ps(t)).is_some() {}
}

/// A power-on low, then `pulses` on the pin from `START`, then low.
fn by_pin(count: u16, pulses: &[(bool, u64)]) -> Core {
    by_pin_into(Core::new(count), pulses).0
}

/// [`by_pin`] on `core`, with each `DOUT` level the core drove and when, as `(ps, high)`.
fn by_pin_into(mut core: Core, pulses: &[(bool, u64)]) -> (Core, Vec<(u64, bool)>) {
    let mut edges = Vec::new();
    let mut high = false;
    let mut settle = |core: &mut Core, t: u64| {
        while let Some(stop) = core.run_until(Time::from_ps(t)) {
            let now = match core.hosted.drive() {
                [] => continue,
                [Drive::High] => true,
                [Drive::Low] => false,
                other => panic!("DOUT drives {other:?}"),
            };
            if now != high {
                edges.push((stop.at.as_ps(), now));
                high = now;
            }
        }
    };
    core.input(Time::ZERO, &[false]);
    let mut t = START;
    for &(level, ps) in pulses {
        settle(&mut core, t);
        core.input(Time::from_ps(t), &[level]);
        t += ps;
    }
    settle(&mut core, t);
    core.input(Time::from_ps(t), &[false]);
    settle(&mut core, t + 2 * RES);
    (core, edges)
}

/// Edges as levels from `START`, the first low until the first edge.
fn levels(edges: &[(u64, bool)]) -> Vec<(bool, u64)> {
    let mut t = START;
    let mut level = false;
    let mut out = Vec::new();
    for &(at, high) in edges {
        assert_ne!(high, level, "DOUT changes at each stop");
        out.push((level, at - t));
        (t, level) = (at, high);
    }
    out.push((level, 1));
    out
}

/// The pin at `idle` from power-on, then each record landing at its end. As at the seam,
/// a landing leaves no stop at or before its time.
fn by_records(count: u16, idle: bool, records: &[(u64, Vec<(bool, u64)>)]) -> Core {
    let mut core = Core::new(count);
    core.input(Time::ZERO, &[idle]);
    let mut last = 0;
    for (start, pulses) in records {
        last = end(*start, pulses);
        settle(&mut core, last);
        core.land(Time::from_ps(*start), idle, pulses, Time::from_ps(last));
        assert_eq!(core.clone().run_until(Time::from_ps(last)), None);
    }
    settle(&mut core, last + 2 * RES);
    core
}

fn latch_times(core: &Core) -> Vec<u64> {
    core.latches().iter().map(|row| row.at.as_ps()).collect()
}

#[test]
fn a_bit_is_told_by_its_high_alone() {
    let bytes = [0xA5, 0x5A, 0xC3];
    let timings = [
        ("the sketch", ZERO, ONE),
        ("an RMT at 10 MHz", (300_000, 900_000), (900_000, 300_000)),
        (
            "Arduino rgbLedWrite",
            (400_000, 800_000),
            (800_000, 400_000),
        ),
        (
            "T0H max over T0L min",
            (400_000, 580_000),
            (850_000, 400_000),
        ),
        (
            "T0H min over T0L max",
            (220_000, 1_000_000),
            (580_000, 220_000),
        ),
        ("T1H max", (220_000, 50_000), (1_000_000, 3_000_000)),
    ];
    for (name, zero, one) in timings {
        let pulses = train(&bytes, zero, one);
        let pin = by_pin(1, &pulses);
        let record = by_records(1, false, &[(START, pulses.clone())]);
        for (path, core) in [("pin", &pin), ("record", &record)] {
            assert_eq!(
                core.observed().pixels,
                vec![[0xA5, 0x5A, 0xC3]],
                "{name} {path}"
            );
            assert_eq!(core.broke(), None, "{name} {path}");
            assert_eq!(
                latch_times(core),
                vec![reset_after(START, &pulses)],
                "{name} {path}"
            );
        }
    }
}

#[test]
fn a_low_of_up_to_280_us_between_bits_ends_the_bit_and_does_not_latch() {
    let mut pulses = train(&[0xFF, 0x00, 0x81], ZERO, ONE);
    pulses[1].1 = 1;
    pulses[3].1 = RES;
    pulses[5].1 = 100_000_000;
    for core in [
        by_pin(1, &pulses),
        by_records(1, false, &[(START, pulses.clone())]),
    ] {
        assert_eq!(core.observed().pixels, vec![[0xFF, 0x00, 0x81]]);
        assert_eq!(core.latches().len(), 1);
        assert_eq!(core.broke(), None);
    }
}

#[test]
fn a_high_outside_both_windows_warns_once_and_drops_the_bit() {
    for high in [219_999, 400_001, 579_999, 1_000_001] {
        let mut pulses = vec![(true, high), (false, ZERO.1)];
        pulses.extend(train(&[0x0F, 0xF0, 0x3C], ZERO, ONE));
        pulses.extend([(true, high), (false, ZERO.1)]);
        for core in [
            by_pin(1, &pulses),
            by_records(1, false, &[(START, pulses.clone())]),
        ] {
            assert_eq!(core.observed().pixels, vec![[0x0F, 0xF0, 0x3C]], "{high}");
            assert_eq!(
                core.broke(),
                Some((ProtocolRule::Bit, Time::from_ps(START + high)))
            );
        }
    }
}

#[test]
fn a_low_of_280_us_does_not_latch_and_one_past_it_does() {
    let mut core = Core::new(1);
    core.input(Time::ZERO, &[false]);
    let mut pulses = train(&[0x80, 0x00, 0x00], ZERO, ONE);
    pulses.pop();
    let fall = end(START, &pulses);
    core.land(Time::from_ps(START), false, &pulses, Time::from_ps(fall));
    assert_eq!(core.run_until(Time::from_ps(fall + RES)), None);
    assert_eq!(core.observed().pixels, vec![[0, 0, 0]]);
    assert!(core.latches().is_empty());
    let stop = core.run_until(Time::from_ps(fall + RES + 1));
    assert_eq!(
        stop.map(|stop| stop.at),
        Some(Time::from_ps(fall + RES + 1))
    );
    assert_eq!(core.observed().pixels, vec![[0x80, 0, 0]]);
    assert_eq!(latch_times(&core), vec![fall + RES + 1]);
}

#[test]
fn a_frame_split_across_two_records_latches_as_one() {
    let bytes: Vec<u8> = (0..13u8).flat_map(|i| [i, i + 16, i + 32]).collect();
    let pulses = train(&bytes, ZERO, ONE);
    let whole = by_records(13, false, &[(START, pulses.clone())]);
    assert_eq!(whole.latches().len(), 1);
    for (idle, at) in [false, true].into_iter().flat_map(|idle| {
        [18 * 8 * 2, 18 * 8 * 2 + 1]
            .into_iter()
            .map(move |at| (idle, at))
    }) {
        let (first, second) = pulses.split_at(at);
        // On an idle-high line a long gap after a high is a long high.
        let gaps: &[u64] = if idle { &[0, 1] } else { &[0, 1, 100_000_000] };
        for &gap in gaps {
            let split = by_records(
                13,
                idle,
                &[
                    (START, first.to_vec()),
                    (end(START, first) + gap, second.to_vec()),
                ],
            );
            let case = format!("idle {idle}, split at pulse {at}, gap {gap}");
            assert_eq!(split.observed(), whole.observed(), "{case}");
            assert_eq!(
                latch_times(&split),
                vec![latch_times(&whole)[0] + gap],
                "{case}"
            );
            assert_eq!(split.broke(), None, "{case}");
        }
    }
}

#[test]
fn a_record_that_starts_at_the_reset_keeps_the_latch_and_one_before_it_does_not() {
    let mut pulses = train(&[1, 2, 3], ZERO, ONE);
    pulses.last_mut().unwrap().1 = ZERO.1;
    let fall = end(START, &pulses) - ZERO.1;
    let latch = fall + RES + 1;
    let next = train(&[4, 5, 6], ZERO, ONE);
    let two = by_records(1, false, &[(START, pulses.clone()), (latch, next.clone())]);
    assert_eq!(latch_times(&two), vec![latch, reset_after(latch, &next)]);
    assert_eq!(two.latches()[0].pixels, vec![[1, 2, 3]]);
    assert_eq!(two.observed().pixels, vec![[4, 5, 6]]);
    let one = by_records(1, false, &[(START, pulses), (latch - 1, next)]);
    assert_eq!(one.latches().len(), 1);
    assert_eq!(one.observed().pixels, vec![[1, 2, 3]]);
}

#[test]
fn a_high_a_record_ends_on_keeps_its_start_on_either_idle_level() {
    let pulses = train(&[0x80, 0, 0], ZERO, ONE);
    let (first, second) = pulses.split_at(1);
    for idle in [false, true] {
        for gap in [0, 1] {
            let split = by_records(
                1,
                idle,
                &[
                    (START, first.to_vec()),
                    (end(START, first) + gap, second.to_vec()),
                ],
            );
            let case = format!("idle {idle}, gap {gap}");
            assert_eq!(split.observed().pixels, vec![[0x80, 0, 0]], "{case}");
            assert_eq!(split.broke(), None, "{case}");
        }
    }
}

#[test]
fn a_low_of_one_past_the_reset_latches_inside_the_record() {
    let mut pulses = train(&[1, 2, 3], ZERO, ONE);
    pulses.last_mut().unwrap().1 = RES + 1;
    let latch = end(START, &pulses);
    let ends = by_records(1, false, &[(START, pulses.clone())]);
    assert_eq!(latch_times(&ends), vec![latch]);
    assert_eq!(ends.observed().pixels, vec![[1, 2, 3]]);

    let next = train(&[4, 5, 6], ZERO, ONE);
    pulses.extend(next.iter().copied());
    let goes_on = by_records(1, false, &[(START, pulses)]);
    assert_eq!(
        latch_times(&goes_on),
        vec![latch, reset_after(latch, &next)]
    );
    assert_eq!(goes_on.latches()[0].pixels, vec![[1, 2, 3]]);
    assert_eq!(goes_on.observed().pixels, vec![[4, 5, 6]]);
}

#[test]
fn an_idle_high_line_times_the_first_high_from_the_record() {
    let pulses = train(&[0xC0, 0xFF, 0x01], ZERO, ONE);
    let next = train(&[0x01, 0x02, 0x03], ZERO, ONE);
    let records = [(START, pulses.clone()), (end(START, &pulses) + RES, next)];
    let core = by_records(1, true, &records);
    assert_eq!(core.observed().pixels, vec![[0x01, 0x02, 0x03]]);
    assert_eq!(core.latches()[0].pixels, vec![[0xC0, 0xFF, 0x01]]);
    assert_eq!(core.broke(), None);
    assert_eq!(
        latch_times(&core),
        latch_times(&by_records(1, false, &records))
    );
}

#[test]
fn a_held_line_falls_with_no_bit_when_a_record_opens_low() {
    let mut opens_low = vec![(false, 500_000)];
    opens_low.extend(train(&[0x80, 0, 0], ZERO, ONE));
    let one = by_records(1, true, &[(START, opens_low.clone())]);
    assert_eq!(one.observed().pixels, vec![[0x80, 0, 0]]);
    assert_eq!(one.broke(), None);
    assert_eq!(latch_times(&one), vec![reset_after(START, &opens_low)]);

    let first = train(&[0x11, 0x22, 0x33], ZERO, ONE);
    opens_low[0].1 = 400_000;
    let next = end(START, &first) + 1_000_000_000;
    let two = by_records(1, true, &[(START, first), (next, opens_low)]);
    assert_eq!(two.latches()[0].pixels, vec![[0x11, 0x22, 0x33]]);
    assert_eq!(two.observed().pixels, vec![[0x80, 0, 0]]);
    assert_eq!(two.broke(), None);
}

#[test]
fn the_line_returns_to_idle_after_a_record() {
    let mut ends_high = train(&[0x12, 0x34, 0x56], ZERO, ONE);
    ends_high.pop();
    let low = by_records(1, false, &[(START, ends_high.clone())]);
    assert_eq!(low.observed().pixels, vec![[0x12, 0x34, 0x56]]);
    assert_eq!(latch_times(&low), vec![end(START, &ends_high) + RES + 1]);

    let mut ends_low = train(&[0x12, 0x34, 0x56], ZERO, ONE);
    ends_low.last_mut().unwrap().1 = ZERO.1;
    let high = by_records(1, true, &[(START, ends_low)]);
    assert!(high.latches().is_empty());
    assert_eq!(high.observed().pixels, vec![[0, 0, 0]]);
    assert_eq!(high.broke(), None);
}

#[test]
fn the_first_byte_is_green() {
    let core = by_pin(1, &train(&[0xA5, 0x00, 0x00], ZERO, ONE));
    assert_eq!(core.observed().pixels, vec![[0xA5, 0, 0]]);
}

#[test]
fn two_leds_take_24_bits_each() {
    let core = by_pin(2, &train(&[1, 2, 3, 4, 5, 6], ZERO, ONE));
    assert_eq!(core.observed().pixels, vec![[1, 2, 3], [4, 5, 6]]);
}

#[test]
fn bits_after_the_chain_are_discarded() {
    let core = by_pin(1, &train(&[1, 2, 3, 0xFF, 0xFF, 0xFF], ZERO, ONE));
    assert_eq!(core.observed().pixels, vec![[1, 2, 3]]);
}

#[test]
fn a_partial_led_keeps_its_colour() {
    let mut core = by_pin(2, &train(&[1, 2, 3, 4, 5, 6], ZERO, ONE));
    let pulses = train(&[7, 8, 9, 10], ZERO, ONE);
    let start = START + 4 * RES;
    core.land(
        Time::from_ps(start),
        false,
        &pulses,
        Time::from_ps(end(start, &pulses)),
    );
    settle(&mut core, end(start, &pulses) + 2 * RES);
    assert_eq!(core.observed().pixels, vec![[7, 8, 9], [4, 5, 6]]);
}

#[test]
fn a_fresh_line_under_an_idle_high_record_keeps_its_first_bit() {
    let pulses = train(&[0x80, 0, 0], ZERO, ONE);
    let mut core = Core::new(1);
    let end = end(START, &pulses);
    core.land(Time::from_ps(START), true, &pulses, Time::from_ps(end));
    settle(&mut core, end + 2 * RES);
    assert_eq!(core.observed().pixels, vec![[0x80, 0, 0]]);
    assert_eq!(core.broke(), None);
}

#[test]
fn a_held_line_that_falls_on_its_pin_ends_no_bit() {
    let mut core = Core::new(1);
    core.input(Time::ZERO, &[true]);
    core.input(Time::from_ps(START), &[false]);
    settle(&mut core, START + RES + 1);
    assert_eq!(core.broke(), None);
    assert_eq!(latch_times(&core), vec![START + RES + 1]);
    assert_eq!(core.observed().pixels, vec![[0, 0, 0]]);

    let mut early = Core::new(1);
    early.input(Time::ZERO, &[true]);
    early.input(Time::from_ps(300_000), &[false]);
    settle(&mut early, 300_000 + RES + 1);
    let pulses = train(&[0x80, 0, 0], ZERO, ONE);
    let mut t = START;
    for &(level, ps) in &pulses {
        settle(&mut early, t);
        early.input(Time::from_ps(t), &[level]);
        t += ps;
    }
    settle(&mut early, t);
    early.input(Time::from_ps(t), &[false]);
    settle(&mut early, t + 2 * RES);
    assert_eq!(early.broke(), None);
    assert_eq!(early.observed().pixels, vec![[0x80, 0, 0]]);
}

/// A session writes no `leds.csv`, so a chain there keeps its last latch alone. Keeping every
/// one made each latch copy all the latches before it, which a checkpoint copy shares, so a
/// long session slowed as it ran. A run that writes the file keeps them all.
#[test]
fn a_chain_that_writes_no_rows_keeps_its_last_latch_alone() {
    let frames = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
    let trains: Vec<_> = frames.iter().map(|f| train(f, ZERO, ONE)).collect();
    let mut starts = vec![START];
    for pulses in &trains {
        starts.push(end(*starts.last().unwrap(), pulses));
    }
    let latched: Vec<u64> = trains
        .iter()
        .zip(&starts)
        .map(|(pulses, &start)| reset_after(start, pulses))
        .collect();
    let pulses = trains.concat();
    let session = Core {
        hosted: super::core(1, 0, None, None, None),
    };
    let (session, _) = by_pin_into(session, &pulses);
    let (run, _) = by_pin_into(Core::new(1), &pulses);
    assert_eq!(session.observed().pixels, vec![[7, 8, 9]]);
    assert_eq!(latch_times(&session), latched[2..]);
    assert_eq!(session.latches()[0].pixels, vec![[7, 8, 9]]);
    assert_eq!(session.hosted.latched(), Some(Time::from_ps(latched[2])));
    assert_eq!(latch_times(&run), latched);
}

/// The sheet's DIN to DOUT delay, 500 ns.
const DELAY: u64 = 500_000;

#[test]
fn a_chain_with_no_dout_drives_nothing() {
    let core = by_pin(1, &train(&[1, 2, 3, 4, 5, 6], ZERO, ONE));
    assert_eq!(core.hosted.drive(), &[] as &[Drive]);
}

#[test]
fn dout_is_low_for_the_chain_s_own_bits_then_follows_din_by_the_delay() {
    let pulses = train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3], ZERO, ONE);
    let (core, edges) = by_pin_into(Core::with_dout(1, DELAY), &pulses);
    assert_eq!(core.observed().pixels, vec![[0x12, 0x34, 0x56]]);
    assert_eq!(core.hosted.drive(), &[Drive::Low]);
    let mut want = Vec::new();
    let mut t = START;
    for (k, &(level, ps)) in pulses.iter().enumerate() {
        if level && k >= 48 {
            want.push((t + DELAY, true));
            want.push((t + ps + DELAY, false));
        }
        t += ps;
    }
    assert_eq!(edges, want);
}

#[test]
fn four_chained_leds_each_take_their_own_24_bits() {
    let colours = [[0xFF, 0, 0], [0, 0xFF, 0], [0, 0, 0xFF], [0x10, 0x20, 0x30]];
    let mut pulses = train(colours.as_flattened(), ZERO, ONE);
    for (k, colour) in colours.iter().enumerate() {
        let (core, edges) = by_pin_into(Core::with_dout(1, DELAY), &pulses);
        assert_eq!(core.observed().pixels, vec![*colour], "led {k}");
        assert_eq!(core.broke(), None, "led {k}");
        pulses = levels(&edges);
    }
    assert_eq!(pulses, vec![(false, 1)], "the last LED passes nothing on");
}

/// The record a core sent, as `(start, idle, pulses)`, from the stop at its end.
fn sent(core: &mut Core, end: u64) -> (u64, bool, Vec<(bool, u64)>) {
    assert_eq!(
        core.run_until(Time::from_ps(end)),
        Some(Stop {
            at: Time::from_ps(end),
            sample: false,
            record: true,
        })
    );
    let mut records = Vec::new();
    core.hosted
        .drain_records(Time::from_ps(end), &mut |record| records.push(record));
    let [record] = &records[..] else {
        panic!("one record, not {records:?}");
    };
    assert_eq!(record.end.as_ps(), end);
    let RecordKind::Pulses { pulses, .. } = &record.kind else {
        panic!("a relay sends pulses");
    };
    (record.start.as_ps(), pulses.idle, pulses.pulses.to_vec())
}

#[test]
fn a_record_past_the_chain_goes_on_as_a_record_and_drives_nothing() {
    let pulses = train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3], ZERO, ONE);
    let mut core = Core::with_dout(1, DELAY);
    core.input(Time::ZERO, &[false]);
    let last = end(START, &pulses);
    settle(&mut core, last);
    core.land(Time::from_ps(START), false, &pulses, Time::from_ps(last));
    assert_eq!(core.hosted.drive(), &[Drive::Low]);

    // The 25th high's rise, 500 ns on, to the 48th high's fall, 500 ns on, then low to 1 ps past
    // the record's end.
    let first_past: u64 = START + pulses[..48].iter().map(|&(_, ps)| ps).sum::<u64>();
    let last_fall = last - pulses.last().unwrap().1;
    let mut want = pulses[48..95].to_vec();
    want.push((false, last + 1 - (last_fall + DELAY)));
    assert_eq!(
        sent(&mut core, last + 1),
        (first_past + DELAY, false, want.clone())
    );
    assert_eq!(core.hosted.drive(), &[Drive::Low]);
    assert_eq!(core.broke(), None);

    settle(&mut core, last + 2 * RES);
    assert_eq!(core.observed().pixels, vec![[0x12, 0x34, 0x56]]);

    let mut next = Core::with_dout(1, DELAY);
    next.input(Time::ZERO, &[false]);
    settle(&mut next, last + 1);
    next.land(
        Time::from_ps(first_past + DELAY),
        false,
        &want,
        Time::from_ps(last + 1),
    );
    while let Some(stop) = next.run_until(Time::from_ps(last + 2 * RES)) {
        assert!(!stop.record, "the second LED holds every bit it took");
    }
    assert_eq!(next.observed().pixels, vec![[0xA5, 0x5A, 0xC3]]);
    assert_eq!(next.broke(), None);
}

#[test]
fn the_draw_follows_a_latch_on_the_pin_at_the_latch() {
    let pulses = train(&[0xFF, 0x00, 0x01, 0x10, 0x20, 0x30], ZERO, ONE);
    let mut core = Core::drawing(2);
    assert_eq!(core.hosted.draw().amps, amps(2, 0));
    core = by_pin_into(core, &pulses).0;
    assert_eq!(
        core.observed().pixels,
        vec![[0xFF, 0x00, 0x01], [0x10, 0x20, 0x30]]
    );
    let lit = 0xFF + 0x01 + 0x10 + 0x20 + 0x30;
    assert!((core.hosted.draw().amps - amps(2, lit)).abs() < 1e-15);
    assert_eq!(core.hosted.draw().holds_above_v, 3.7);
}

#[test]
fn a_record_that_latches_draws_from_1_ps_after_it_lands_and_a_repeat_adds_no_stop() {
    let pulses = train(&[0xFF, 0x00, 0x01], ZERO, ONE);
    let mut core = Core::drawing(1);
    core.input(Time::ZERO, &[false]);
    let last = end(START, &pulses);
    settle(&mut core, last);
    core.land(Time::from_ps(START), false, &pulses, Time::from_ps(last));
    assert_eq!(core.hosted.draw().amps, amps(1, 0));
    let drawn = core.run_until(Time::from_ps(last + RES));
    assert_eq!(
        drawn,
        Some(Stop {
            at: Time::from_ps(last + 1),
            sample: false,
            record: false,
        })
    );
    assert!((core.hosted.draw().amps - amps(1, 0x100)).abs() < 1e-15);
    assert_eq!(core.run_until(Time::from_ps(last + 2 * RES)), None);

    let start = last + 2 * RES;
    let again = end(start, &pulses);
    settle(&mut core, again);
    core.land(Time::from_ps(start), false, &pulses, Time::from_ps(again));
    assert_eq!(core.run_until(Time::from_ps(again + 2 * RES)), None);
    assert!((core.hosted.draw().amps - amps(1, 0x100)).abs() < 1e-15);
}

fn min_supply(volts: f64) -> crate::netlist::Value {
    crate::netlist::Value {
        si: crate::finite::Finite::new(volts).unwrap(),
    }
}

impl Core {
    fn with_supply(count: u16, delay: u64, with_draw: bool, volts: f64) -> Core {
        let dout = (delay > 0).then_some(NetId::GROUND);
        let draw = with_draw.then(draw);
        let supply = min_supply(volts);
        let mut hosted = super::core(count, delay, dout, draw.as_ref(), Some(&supply));
        hosted.keep_shown();
        Core { hosted }
    }
}

#[test]
fn an_unpowered_chain_ignores_pin_edges_and_records_without_warning() {
    let mut pulses = vec![(true, 450_000), (false, ZERO.1)];
    pulses.extend(train(&[0x12, 0x34, 0x56, 0xA5, 0x5A, 0xC3], ZERO, ONE));

    let mut pin = Core::with_supply(1, DELAY, true, 3.0);
    assert_eq!(pin.hosted.min_supply_v(), Some(3.0));
    pin.input(Time::ZERO, &[false, false]);
    let mut t = START;
    for &(level, ps) in &pulses {
        settle(&mut pin, t);
        pin.input(Time::from_ps(t), &[level, false]);
        t += ps;
    }
    settle(&mut pin, t + 2 * RES);
    assert_eq!(pin.observed().pixels, vec![[0, 0, 0]]);
    assert!(pin.latches().is_empty());
    assert_eq!(pin.broke(), None);

    let mut rec = Core::with_supply(1, DELAY, true, 3.0);
    rec.input(Time::ZERO, &[false, false]);
    let last = end(START, &pulses);
    settle(&mut rec, last);
    rec.land(Time::from_ps(START), false, &pulses, Time::from_ps(last));
    while let Some(stop) = rec.run_until(Time::from_ps(last + 2 * RES)) {
        assert!(!stop.record, "unpowered chain must not relay records");
    }
    assert_eq!(rec.observed().pixels, vec![[0, 0, 0]]);
    assert!(rec.latches().is_empty());
    assert_eq!(rec.broke(), None);
}

#[test]
fn a_chain_picks_up_from_the_first_full_frame_after_power_returns() {
    let first = train(&[0x11, 0x22, 0x33], ZERO, ONE);
    let second = train(&[0x44, 0x55, 0x66], ZERO, ONE);
    let power_on = end(START, &first) + RES;
    let second_start = power_on + 1_000_000;

    // Pin path: first frame ignored while unpowered, second frame latches after power returns.
    let mut pin = Core::with_supply(1, 0, false, 3.0);
    pin.input(Time::ZERO, &[false, false]);
    let mut t = START;
    for &(level, ps) in &first {
        settle(&mut pin, t);
        pin.input(Time::from_ps(t), &[level, false]);
        t += ps;
    }
    settle(&mut pin, power_on);
    pin.input(Time::from_ps(power_on), &[false, true]);
    t = second_start;
    for &(level, ps) in &second {
        settle(&mut pin, t);
        pin.input(Time::from_ps(t), &[level, true]);
        t += ps;
    }
    settle(&mut pin, t + 2 * RES);
    assert_eq!(pin.observed().pixels, vec![[0x44, 0x55, 0x66]]);
    assert_eq!(latch_times(&pin), vec![reset_after(second_start, &second)]);
    assert_eq!(pin.broke(), None);

    // Record path: a record that started before power returned is ignored, and the next full
    // frame after power-up lights the pixels.
    let mut combined = vec![(true, 450_000), (false, ZERO.1)];
    let prefix_end = end(START, &combined);
    let lead_low = 1_000_000;
    combined.push((false, lead_low));
    combined.extend_from_slice(&second);
    let last = end(START, &combined);

    let mut rec = Core::with_supply(1, 0, false, 3.0);
    rec.input(Time::ZERO, &[false, false]);
    settle(&mut rec, prefix_end + lead_low / 2);
    rec.input(Time::from_ps(prefix_end + lead_low / 2), &[false, true]);
    settle(&mut rec, last);
    rec.land(Time::from_ps(START), false, &combined, Time::from_ps(last));
    settle(&mut rec, last + 2 * RES);
    assert_eq!(rec.observed().pixels, vec![[0, 0, 0]]);
    assert!(rec.latches().is_empty());
    assert_eq!(rec.broke(), None);

    let next_start = last + 2 * RES;
    let next_end = end(next_start, &second);
    settle(&mut rec, next_end);
    rec.land(
        Time::from_ps(next_start),
        false,
        &second,
        Time::from_ps(next_end),
    );
    settle(&mut rec, next_end + 2 * RES);
    assert_eq!(rec.observed().pixels, vec![[0x44, 0x55, 0x66]]);
    assert_eq!(latch_times(&rec), vec![reset_after(next_start, &second)]);
    assert_eq!(rec.broke(), None);
}

#[test]
fn power_loss_clears_latched_colours_and_lit_draw_and_recovers_on_next_frame() {
    let first = train(&[0xFF, 0x80, 0x40], ZERO, ONE);
    let second = train(&[0x10, 0x20, 0x30], ZERO, ONE);
    let mut core = Core::with_supply(1, DELAY, true, 3.0);
    core.input(Time::ZERO, &[false, true]);
    let first_end = end(START, &first);
    settle(&mut core, first_end);
    core.land(
        Time::from_ps(START),
        false,
        &first,
        Time::from_ps(first_end),
    );
    settle(&mut core, first_end + 2 * RES);
    assert_eq!(core.observed().pixels, vec![[0xFF, 0x80, 0x40]]);
    assert!((core.hosted.draw().amps - amps(1, 0xFF + 0x80 + 0x40)).abs() < 1e-15);

    let drop_at = first_end + 2 * RES;
    core.input(Time::from_ps(drop_at), &[false, false]);
    assert_eq!(core.observed().pixels, vec![[0, 0, 0]]);
    let redraw = core.run_until(Time::from_ps(drop_at + 10));
    assert_eq!(redraw.map(|s| s.at), Some(Time::from_ps(drop_at + 1)));
    assert!((core.hosted.draw().amps - amps(1, 0)).abs() < 1e-15);

    let restore_at = drop_at + 1_000_000;
    settle(&mut core, restore_at);
    core.input(Time::from_ps(restore_at), &[false, true]);
    let second_start = restore_at + 1_000_000;
    let second_end = end(second_start, &second);
    settle(&mut core, second_end);
    core.land(
        Time::from_ps(second_start),
        false,
        &second,
        Time::from_ps(second_end),
    );
    settle(&mut core, second_end + 2 * RES);
    assert_eq!(core.observed().pixels, vec![[0x10, 0x20, 0x30]]);
    assert!((core.hosted.draw().amps - amps(1, 0x10 + 0x20 + 0x30)).abs() < 1e-15);
    assert_eq!(core.broke(), None);
}
