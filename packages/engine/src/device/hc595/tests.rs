//! The shift register through its host core: edges, the storage register, `OE`,
//! clear, and a second register fed from `QHp`.

use orbe_model::{Device, Drive, Fields, Time};

use super::Quiet;
use super::model::{self, DISABLE_PS, ENABLE_PS, Hc595 as DeviceHc595, LATCH_PS, SHIFT_PS};
use crate::active::native::{Hosted, Native};
use crate::world::World;

type Core = Native<DeviceHc595, Quiet>;

const SER: usize = model::WATCH_SER;
const SRCLK: usize = model::WATCH_SRCLK;
const RCLK: usize = model::WATCH_RCLK;
const SRCLR: usize = model::WATCH_SRCLR;
const OE: usize = model::WATCH_OE;

fn levels(ser: bool, srclk: bool, rclk: bool, srclr: bool, oe: bool) -> [bool; 5] {
    let mut levels = [false; 5];
    levels[SER] = ser;
    levels[SRCLK] = srclk;
    levels[RCLK] = rclk;
    levels[SRCLR] = srclr;
    levels[OE] = oe;
    levels
}

struct Host {
    core: Core,
    world: World,
    now: u64,
    pins: [bool; 5],
}

impl Host {
    fn new() -> Host {
        let world = World::default();
        let mut core = Native::new(
            DeviceHc595::new(&Fields::NONE).unwrap(),
            Quiet,
            model::OUTPUTS,
        );
        let pins = levels(false, false, false, true, false);
        core.input(&world, Time::ZERO, &pins);
        Host {
            core,
            world,
            now: 0,
            pins,
        }
    }

    fn run(&mut self, to: u64) {
        while self
            .core
            .run_until(&self.world, Time::from_ps(to))
            .is_some()
        {}
        self.now = to;
    }

    fn set(&mut self, at: u64, pins: [bool; 5]) {
        assert!(at >= self.now, "time moves forward");
        self.run(at);
        self.core.input(&self.world, Time::from_ps(at), &pins);
        self.pins = pins;
        self.now = at;
    }

    fn clock(&mut self, at: u64, ser: bool) {
        let mut high = self.pins;
        high[SER] = ser;
        high[SRCLK] = false;
        self.set(at, high);
        high[SRCLK] = true;
        self.set(at + 1_000, high);
        high[SRCLK] = false;
        self.set(at + 2_000, high);
    }

    fn latch(&mut self, at: u64) {
        let mut pins = self.pins;
        pins[RCLK] = true;
        self.set(at, pins);
        pins[RCLK] = false;
        self.set(at + 1_000, pins);
    }

    fn drives(&mut self, at: u64) -> [Drive; model::OUTPUTS] {
        self.run(at);
        let mut drives = [Drive::Off; model::OUTPUTS];
        for (index, drive) in drives.iter_mut().enumerate() {
            *drive = self.core.drive()[index];
        }
        drives
    }
}

fn settled(host: &mut Host) -> [Drive; model::OUTPUTS] {
    host.drives(host.now + SHIFT_PS + LATCH_PS + DISABLE_PS)
}

#[test]
fn a_shift_stays_off_the_pins_until_the_latch() {
    let mut host = Host::new();
    assert_eq!(host.drives(1_000), [Drive::Low; model::OUTPUTS]);
    host.clock(100_000, true);
    let after_shift = host.drives(host.now + SHIFT_PS);
    assert_eq!(after_shift[model::QA], Drive::Low);
    assert_eq!(after_shift[model::QHP], Drive::Low);
    host.latch(host.now + 100_000);
    let latched = settled(&mut host);
    assert_eq!(latched[model::QA], Drive::High);
    assert_eq!(latched[model::QH], Drive::Low);
    assert_eq!(latched[model::QHP], Drive::Low);
}

#[test]
fn qhp_feeds_the_next_register_on_the_following_clock() {
    let mut first = Host::new();
    let mut second = Host::new();
    // One 1, then zeros. QHp rises after the 8th edge, and the next register
    // samples the level QHp held at its own edge, which is the previous one.
    for n in 0..9 {
        let at = 100_000 + n * 100_000;
        let into_second = first.drives(at)[model::QHP] == Drive::High;
        first.clock(at, n == 0);
        second.clock(at, into_second);
    }
    assert_eq!(settled(&mut first)[model::QHP], Drive::Low);
    second.latch(second.now + 100_000);
    assert_eq!(settled(&mut second)[model::QA], Drive::High);
}

#[test]
fn tied_clocks_latch_the_shift_register_from_before_the_edge() {
    let mut host = Host::new();
    host.clock(10_000, true);
    host.run(host.now + SHIFT_PS);
    let mut pins = host.pins;
    pins[SER] = false;
    pins[SRCLK] = true;
    pins[RCLK] = true;
    host.set(host.now + 10_000, pins);
    let latched = settled(&mut host);
    assert_eq!(latched[model::QA], Drive::High);
    assert_eq!(latched[model::QHP], Drive::Low);
}

#[test]
fn output_enable_releases_the_pins_and_clear_drops_qhp() {
    let mut host = Host::new();
    for _ in 0..8 {
        host.clock(host.now + 100_000, true);
    }
    host.latch(host.now + 100_000);
    assert!(settled(&mut host).iter().all(|drive| *drive == Drive::High));
    let mut pins = host.pins;
    pins[OE] = true;
    host.set(host.now + 10_000, pins);
    let off = host.drives(host.now + DISABLE_PS);
    assert!(off[..8].iter().all(|drive| *drive == Drive::Off));
    assert_eq!(off[model::QHP], Drive::High);
    pins[OE] = false;
    host.set(host.now + 10_000, pins);
    assert_eq!(host.drives(host.now + ENABLE_PS)[model::QA], Drive::High);
    pins[SRCLR] = false;
    host.set(host.now + 10_000, pins);
    let cleared = settled(&mut host);
    assert_eq!(cleared[model::QHP], Drive::Low);
    assert_eq!(cleared[model::QA], Drive::High);
}

#[test]
fn a_clock_while_clear_is_low_does_not_shift() {
    let mut host = Host::new();
    let mut pins = host.pins;
    pins[SRCLR] = false;
    host.set(10_000, pins);
    host.run(host.now + 20_000);
    host.clock(host.now + 10_000, true);
    host.latch(host.now + 10_000);
    assert_eq!(settled(&mut host), [Drive::Low; model::OUTPUTS]);
}
