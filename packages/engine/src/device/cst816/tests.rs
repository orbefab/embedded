use crate::world::World;
use orbe_model::{Drive, Time, Value};

use super::model::{CHIP_ID, INT_PULSE_PS, REG_CHIP, REG_FINGERS};
use super::*;
use crate::active::i2c::DATA_HOLD_PS;
use crate::active::native::Hosted;

type Core = Native<model::Cst816, Engine>;

const IMAGE_SDA: usize = 0;
const IMAGE_INT: usize = 1;

/// SDA and INT, released.
const RELEASED: [Drive; 2] = [Drive::Off, Drive::Off];

struct Touch {
    x: u16,
    y: u16,
    down: bool,
}

fn power_on() -> Core {
    let mut core = Native::new(
        model::Cst816::new(&Fields::NONE).unwrap(),
        Engine::new(BUS),
        model::OUTPUTS,
    );
    core.input(&World::default(), Time::ZERO, &[true, true, true, true]);
    core
}

/// When the I2C engine first left SDA unmoved because SCL was high.
fn sda_blocked_at(core: &Core) -> Option<Time> {
    core.broke()
        .into_iter()
        .find(|&(rule, _)| rule == crate::diagnostic::ProtocolRule::SdaWhileSclHigh)
        .map(|(_, at)| at)
}

/// The touch at `at`, as the netlist's `touches` field delivers it.
fn touch(core: &mut Core, at: Time, touch: Touch) {
    let fields = vec![
        ("x".to_string(), Value::Int(touch.x.into())),
        ("y".to_string(), Value::Int(touch.y.into())),
        ("down".to_string(), Value::Bool(touch.down)),
    ];
    let event = EventSpec::declared(&TOUCH, &fields).unwrap();
    core.event(&World::default(), at, event.event());
}

/// Master levels, and the wire SDA after the part's own drive.
struct Bus {
    core: Core,
    t: u64,
    scl: bool,
    master_sda: bool,
    rst_high: bool,
    /// `(t_ps, scl, sda driven low)` whenever either line changes.
    trace: Vec<(u64, bool, bool)>,
}

impl Bus {
    fn new() -> Bus {
        Bus {
            core: power_on(),
            t: 0,
            scl: true,
            master_sda: true,
            rst_high: true,
            trace: Vec::new(),
        }
    }

    fn mark(&mut self) {
        let row = (self.t, self.scl, self.sda_low());
        if self.trace.last().is_none_or(|prev| *prev != row) {
            self.trace.push(row);
        }
    }

    fn sda_moved_while_scl_high(&self) -> Option<u64> {
        let mut driven = false;
        for &(t, scl, sda_low) in &self.trace {
            if sda_low != driven && scl {
                return Some(t);
            }
            driven = sda_low;
        }
        None
    }

    fn wire_sda(&self) -> bool {
        self.master_sda && self.core.drive()[IMAGE_SDA] != Drive::Low
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            let drive = self.core.drive()[IMAGE_SDA];
            let sda = self.wire_sda();
            self.core.input(
                &World::default(),
                Time::from_ps(self.t),
                &[self.scl, sda, true, self.rst_high],
            );
            self.mark();
            let limit = self.t.saturating_add(DATA_HOLD_PS + 2);
            while let Some(stop) = self.core.run_until(&World::default(), Time::from_ps(limit)) {
                self.t = stop.at.as_ps();
                self.mark();
            }
            self.t = limit;
            if self.core.drive()[IMAGE_SDA] == drive && sda == self.wire_sda() {
                return;
            }
        }
        panic!("the bus did not settle at {}", self.t);
    }

    fn set(&mut self, t: u64, scl: bool, master_sda: bool) {
        self.t = t;
        self.scl = scl;
        self.master_sda = master_sda;
        let sda = self.wire_sda();
        self.core.input(
            &World::default(),
            Time::from_ps(t),
            &[scl, sda, true, self.rst_high],
        );
        self.mark();
    }

    fn spin(&mut self, limit: u64) {
        while let Some(stop) = self.core.run_until(&World::default(), Time::from_ps(limit)) {
            self.t = stop.at.as_ps();
            self.mark();
            let sda = self.wire_sda();
            self.core.input(
                &World::default(),
                Time::from_ps(self.t),
                &[self.scl, sda, true, self.rst_high],
            );
        }
        self.t = limit;
    }

    fn bit_span(&mut self, high: bool, low_ps: u64) {
        let fall = self.t + 1_000;
        self.set(fall, false, high);
        self.spin(fall + low_ps);
        self.set(fall + low_ps, true, high);
    }

    fn fast_start(&mut self) {
        let t = self.t + FAST_LOW_PS;
        self.set(t, true, true);
        self.set(t + FAST_LOW_PS, true, false);
    }

    fn fast_byte(&mut self, byte: u8) {
        for shift in (0..8).rev() {
            self.bit_span((byte >> shift) & 1 == 1, FAST_LOW_PS);
        }
    }

    fn fast_ack(&mut self) {
        let fall = self.t + 1_000;
        self.set(fall, false, true);
        self.spin(fall + FAST_LOW_PS);
        self.set(fall + FAST_LOW_PS, true, true);
    }

    fn fast_read_byte(&mut self) -> u8 {
        let mut byte = 0u8;
        for shift in (0..8).rev() {
            let fall = self.t + 1_000;
            self.set(fall, false, true);
            self.spin(fall + FAST_LOW_PS);
            self.set(fall + FAST_LOW_PS, true, true);
            if !self.sda_low() {
                byte |= 1 << shift;
            }
        }
        byte
    }

    fn fast_master_ack(&mut self, ack: bool) {
        let fall = self.t + 1_000;
        self.set(fall, false, !ack);
        self.spin(fall + FAST_LOW_PS);
        self.set(fall + FAST_LOW_PS, true, !ack);
    }

    fn fast_repeated_start(&mut self) {
        let fall = self.t + 1_000;
        self.set(fall, false, true);
        self.spin(fall + FAST_LOW_PS);
        let rise = fall + FAST_LOW_PS;
        self.set(rise, true, true);
        self.set(rise + FAST_LOW_PS, true, false);
    }

    fn at(&mut self, scl: bool, master_sda: bool) {
        self.t += 1_000;
        self.scl = scl;
        self.master_sda = master_sda;
        self.settle();
    }

    /// One bit, MSB first, with the master driving `high` while SCL is low and then high.
    fn bit(&mut self, high: bool) {
        self.at(false, high);
        self.at(true, high);
    }

    fn byte(&mut self, byte: u8) {
        for shift in (0..8).rev() {
            self.bit((byte >> shift) & 1 == 1);
        }
    }

    /// The fall that opens the ACK, then the ACK high, with the master released.
    fn ack_clock(&mut self) {
        self.at(false, true);
        self.at(true, true);
    }

    fn start(&mut self) {
        self.at(true, true);
        self.at(true, false);
    }

    fn stop(&mut self) {
        self.at(false, false);
        self.at(true, false);
        self.at(true, true);
    }

    /// SCL falls, SDA is high, SCL rises, SDA falls.
    fn repeated_start(&mut self) {
        self.at(false, true);
        self.at(true, true);
        self.at(true, false);
    }

    /// The master ACKs when `ack` is set, and NACKs otherwise.
    fn master_ack(&mut self, ack: bool) {
        self.at(false, !ack);
        self.at(true, !ack);
    }

    /// The eight bits the part is driving, MSB first.
    fn read_byte(&mut self) -> u8 {
        let mut byte = 0u8;
        for shift in (0..8).rev() {
            self.at(false, true);
            self.at(true, true);
            if !self.sda_low() {
                byte |= 1 << shift;
            }
        }
        byte
    }

    fn sda_low(&self) -> bool {
        self.core.drive()[IMAGE_SDA] == Drive::Low
    }
}

#[test]
fn a_matching_address_is_acked_and_another_is_not() {
    let mut bus = Bus::new();
    bus.start();
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    assert!(bus.sda_low(), "0x15 write is ACKed");
    bus.stop();
    assert!(!bus.sda_low(), "STOP releases SDA");

    let mut bus = Bus::new();
    bus.start();
    bus.byte(0x16 << 1);
    bus.ack_clock();
    assert!(!bus.sda_low(), "0x16 is not ACKed");
}

#[test]
fn a_read_returns_the_byte_at_the_pointer_and_the_next() {
    let mut bus = Bus::new();
    touch(
        &mut bus.core,
        Time::from_ps(500),
        Touch {
            x: 0x123,
            y: 0x045,
            down: true,
        },
    );
    bus.core.run_until(&World::default(), Time::from_ps(501));
    bus.start();
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    bus.byte(REG_FINGERS as u8);
    bus.ack_clock();
    bus.repeated_start();
    bus.byte((ADDRESS << 1) | 1);
    bus.ack_clock();
    let mut got = [0u8; 5];
    for index in 0..got.len() {
        got[index] = bus.read_byte();
        bus.master_ack(index + 1 != got.len());
    }
    assert_eq!(got, [1, 0x01, 0x23, 0x00, 0x45]);
}

#[test]
fn the_chip_id_is_the_byte_at_0xa7() {
    let mut bus = Bus::new();
    bus.start();
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    bus.byte(REG_CHIP as u8);
    bus.ack_clock();
    bus.repeated_start();
    bus.byte((ADDRESS << 1) | 1);
    bus.ack_clock();
    assert_eq!(bus.read_byte(), CHIP_ID);
}

#[test]
fn a_touch_leaves_the_image_and_pulses_int() {
    let mut core = power_on();
    let at = Time::from_ps(5_000);
    touch(
        &mut core,
        at,
        Touch {
            x: 10,
            y: 20,
            down: true,
        },
    );
    assert_eq!(core.drive(), &RELEASED);
    assert_eq!(
        core.run_until(&World::default(), at),
        None,
        "no drive at the touch"
    );
    let fall = Time::from_ps(at.as_ps() + 1);
    let stop = core.run_until(&World::default(), fall).expect("INT falls");
    assert_eq!(stop.at, fall);
    assert_eq!(core.drive()[IMAGE_INT], Drive::Low);
    let rise = Time::from_ps(fall.as_ps() + INT_PULSE_PS);
    assert_eq!(
        core.run_until(&World::default(), Time::from_ps(rise.as_ps() - 1)),
        None
    );
    let stop = core.run_until(&World::default(), rise).expect("INT rises");
    assert_eq!(stop.at, rise);
    assert_eq!(core.drive()[IMAGE_INT], Drive::Off);
}

#[test]
fn reset_releases_int_and_clears_a_point() {
    let mut bus = Bus::new();
    touch(
        &mut bus.core,
        Time::from_ps(1_000),
        Touch {
            x: 1,
            y: 2,
            down: true,
        },
    );
    bus.core.run_until(&World::default(), Time::from_ps(1_001));
    assert_eq!(bus.core.drive()[IMAGE_INT], Drive::Low);
    bus.rst_high = false;
    bus.set(2_000, true, true);
    let release = Time::from_ps(2_001);
    let stop = bus
        .core
        .run_until(&World::default(), release)
        .expect("INT releases");
    assert_eq!(stop.at, release);
    assert_eq!(bus.core.drive(), &RELEASED);
    bus.rst_high = true;
    bus.set(3_000, true, true);
    assert_eq!(
        read_from(&mut bus, REG_FINGERS as u8, 5, true),
        [0, 0, 0, 0, 0]
    );
    assert_eq!(read_from(&mut bus, REG_CHIP as u8, 1, true), [CHIP_ID]);
}

const FAST_LOW_PS: u64 = 450_000;

fn read_from(bus: &mut Bus, pointer: u8, n: usize, nack_last: bool) -> Vec<u8> {
    bus.start();
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    bus.byte(pointer);
    bus.ack_clock();
    bus.repeated_start();
    bus.byte((ADDRESS << 1) | 1);
    bus.ack_clock();
    let mut got = Vec::with_capacity(n);
    for index in 0..n {
        got.push(bus.read_byte());
        bus.master_ack(!nack_last || index + 1 != n);
    }
    got
}

#[test]
fn a_finger_up_keeps_the_last_point() {
    let mut bus = Bus::new();
    touch(
        &mut bus.core,
        Time::from_ps(500),
        Touch {
            x: 291,
            y: 69,
            down: true,
        },
    );
    bus.core.run_until(&World::default(), Time::from_ps(501));
    touch(
        &mut bus.core,
        Time::from_ps(3_000),
        Touch {
            x: 0,
            y: 0,
            down: false,
        },
    );
    bus.core.run_until(&World::default(), Time::from_ps(3_001));
    bus.t = 3_001;
    assert_eq!(
        read_from(&mut bus, REG_FINGERS as u8, 5, true),
        [0, 0x01, 0x23, 0x00, 0x45]
    );
}

#[test]
fn a_write_is_what_the_next_read_returns() {
    let mut bus = Bus::new();
    bus.start();
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    bus.byte(0x10);
    bus.ack_clock();
    bus.byte(0x5A);
    bus.ack_clock();
    bus.byte(0x5B);
    bus.ack_clock();
    bus.stop();
    assert_eq!(read_from(&mut bus, 0x10, 2, true), [0x5A, 0x5B]);
}

#[test]
fn a_stop_inside_a_byte_drops_the_rest() {
    let mut bus = Bus::new();
    bus.start();
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    bus.bit(false);
    bus.bit(false);
    bus.bit(false);
    bus.at(true, true);
    assert!(!bus.sda_low(), "STOP releases SDA");
    for _ in 0..5 {
        bus.bit(true);
    }
    bus.ack_clock();
    assert!(!bus.sda_low(), "the byte that STOP cut is not ACKed");
}

#[test]
fn a_start_inside_a_byte_addresses_the_part() {
    let mut bus = Bus::new();
    bus.start();
    bus.bit(false);
    bus.bit(false);
    bus.bit(true);
    bus.at(true, false);
    bus.byte(ADDRESS << 1);
    bus.ack_clock();
    assert!(bus.sda_low(), "the address after the new START is ACKed");
    bus.byte(REG_CHIP as u8);
    bus.ack_clock();
    bus.repeated_start();
    bus.byte((ADDRESS << 1) | 1);
    bus.ack_clock();
    assert_eq!(bus.read_byte(), CHIP_ID);
}

#[test]
fn reset_during_a_transfer_releases_sda_after_the_hold() {
    let mut bus = Bus::new();
    touch(
        &mut bus.core,
        Time::from_ps(1_000),
        Touch {
            x: 4,
            y: 5,
            down: true,
        },
    );
    bus.core.run_until(&World::default(), Time::from_ps(1_001));
    assert_eq!(bus.core.drive()[IMAGE_INT], Drive::Low);
    bus.t = 1_001;
    bus.start();
    bus.byte(ADDRESS << 1);
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.spin(fall + DATA_HOLD_PS);
    assert!(bus.sda_low(), "the address ACK is driven");
    let rst_at = bus.t + 1_000;
    bus.rst_high = false;
    bus.set(rst_at, false, true);
    bus.spin(rst_at + 1);
    assert_eq!(
        bus.core.drive()[IMAGE_INT],
        Drive::Off,
        "INT releases 1 ps later"
    );
    assert!(bus.sda_low(), "SDA stays down through that picosecond");
    bus.spin(rst_at + DATA_HOLD_PS);
    assert!(!bus.sda_low(), "SDA releases after the hold");
    bus.rst_high = true;
    bus.set(bus.t + 1_000, true, true);
    assert_eq!(
        read_from(&mut bus, REG_FINGERS as u8, 5, true),
        [0, 0, 0, 0, 0]
    );
    assert_eq!(read_from(&mut bus, REG_CHIP as u8, 1, true), [CHIP_ID]);
}

#[test]
fn sda_changes_only_while_scl_is_low() {
    let mut bus = Bus::new();
    bus.start();
    bus.byte(ADDRESS << 1);
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.set(fall + 100_000, true, true);
    bus.spin(fall + DATA_HOLD_PS + 2);
    assert!(!bus.sda_low(), "a 100 ns low leaves the ACK undriven");
    let first = sda_blocked_at(&bus.core).expect("the rule is recorded");
    assert_eq!(first.as_ps(), fall + DATA_HOLD_PS);
    bus.stop();
    bus.start();
    bus.byte(ADDRESS << 1);
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.set(fall + 100_000, true, true);
    bus.spin(fall + DATA_HOLD_PS + 2);
    assert_eq!(
        sda_blocked_at(&bus.core),
        Some(first),
        "the warning stays at the first time"
    );
    assert_eq!(bus.sda_moved_while_scl_high(), None);
}

#[test]
fn a_450_ns_low_ends_a_read_on_the_master_nack() {
    let mut bus = Bus::new();
    bus.fast_start();
    bus.fast_byte(ADDRESS << 1);
    bus.fast_ack();
    bus.fast_byte(REG_CHIP as u8);
    bus.fast_ack();
    bus.fast_repeated_start();
    bus.fast_byte((ADDRESS << 1) | 1);
    bus.fast_ack();
    let id = bus.fast_read_byte();
    bus.fast_master_ack(true);
    let next = bus.fast_read_byte();
    bus.fast_master_ack(false);
    assert_eq!(id, CHIP_ID);
    assert_eq!(next, 0, "the byte after the chip id");
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.spin(fall + FAST_LOW_PS);
    assert!(!bus.sda_low(), "the NACK ended the read");
    assert_eq!(bus.sda_moved_while_scl_high(), None);
    assert_eq!(sda_blocked_at(&bus.core), None);
}

#[test]
fn reset_while_scl_is_high_releases_sda_and_logs_nothing() {
    let mut bus = Bus::new();
    touch(
        &mut bus.core,
        Time::from_ps(1_000),
        Touch {
            x: 4,
            y: 5,
            down: true,
        },
    );
    bus.core.run_until(&World::default(), Time::from_ps(1_001));
    assert_eq!(bus.core.drive()[IMAGE_INT], Drive::Low);
    bus.t = 1_001;
    bus.start();
    bus.byte(ADDRESS << 1);
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.spin(fall + DATA_HOLD_PS);
    assert!(bus.sda_low(), "the address ACK is driven");
    let rise = bus.t + 1_000;
    bus.set(rise, true, true);
    assert!(bus.sda_low(), "the ACK stays driven while SCL is high");
    let rst_at = bus.t + 1_000;
    bus.rst_high = false;
    bus.set(rst_at, true, true);
    bus.spin(rst_at + 1);
    assert_eq!(
        bus.core.drive()[IMAGE_INT],
        Drive::Off,
        "INT releases 1 ps later"
    );
    assert!(bus.sda_low(), "SDA stays down through that picosecond");
    bus.spin(rst_at + DATA_HOLD_PS);
    assert!(!bus.sda_low(), "SDA releases while SCL is high");
    assert_eq!(sda_blocked_at(&bus.core), None, "reset logs no warning");
}

fn lets_go_and_stops(bus: &mut Bus) {
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.spin(fall + DATA_HOLD_PS + 2);
    assert!(!bus.sda_low(), "the target let go of SDA");
    bus.set(bus.t + 1_000, false, false);
    bus.set(bus.t + 1_000, true, false);
    bus.set(bus.t + 1_000, true, true);
    assert!(!bus.sda_low(), "the master's STOP is on the wire");
    assert_eq!(
        read_from(bus, REG_CHIP as u8, 1, true),
        [CHIP_ID],
        "the next START is a new transfer"
    );
}

#[test]
fn a_refused_release_before_a_nack_does_not_read_on() {
    let mut bus = Bus::new();
    bus.fast_start();
    bus.fast_byte(ADDRESS << 1);
    bus.fast_ack();
    bus.fast_byte(REG_CHIP as u8);
    bus.fast_ack();
    bus.fast_repeated_start();
    bus.fast_byte((ADDRESS << 1) | 1);
    bus.fast_ack();
    assert_eq!(bus.fast_read_byte(), CHIP_ID);
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.set(fall + 100_000, true, true);
    bus.spin(fall + DATA_HOLD_PS + 2);
    assert!(bus.sda_low(), "the release is still the target's");
    let blocked = sda_blocked_at(&bus.core).expect("the rule is recorded");
    assert_eq!(blocked.as_ps(), fall + DATA_HOLD_PS);
    lets_go_and_stops(&mut bus);
    assert_eq!(sda_blocked_at(&bus.core), Some(blocked));
}

#[test]
fn a_refused_address_ack_does_not_read_on() {
    let mut bus = Bus::new();
    bus.fast_start();
    bus.fast_byte((ADDRESS << 1) | 1);
    let fall = bus.t + 1_000;
    bus.set(fall, false, true);
    bus.set(fall + 100_000, true, true);
    bus.spin(fall + DATA_HOLD_PS + 2);
    assert!(!bus.sda_low(), "the ACK stays undriven");
    assert!(sda_blocked_at(&bus.core).is_some(), "the rule is recorded");
    lets_go_and_stops(&mut bus);
}
