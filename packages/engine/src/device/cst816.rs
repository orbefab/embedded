//! The `cst816` touch controller, an active chip with no clock of its own. It is an I2C target
//! at address 0x15. SDA and INT are open drain. SCL is an input, and the part does not hold it.
//! RST low clears the register file and releases both pins.
//!
//! A touch is applied by [`Core::touch`] at the schedule's time. The drive image does not change
//! then. INT falls 1 ps later and rises after [`INT_PULSE_PS`]. That width is a guess.

use crate::netlist::PartId;

use crate::netlist::Event;
use crate::netlist::MIN_GAP_PS;
use crate::netlist::NetId;
use crate::netlist::Threshold;
use crate::netlist::Value;

use crate::time::Time;

use crate::active::i2c::{self, Engine, Target};
use crate::active::{Chip, Drive, Output, Stage, Stop};
use crate::mna::Node;
use crate::watch::{Signal, Watch};

const VCC: usize = 0;
const GND: usize = 1;
const SCL: usize = 2;
const SDA: usize = 3;
const INT_PIN: usize = 4;
const RST: usize = 5;

/// Watch order: SCL, SDA, INT, RST.
const WATCH_SCL: usize = 0;
const WATCH_SDA: usize = 1;
const WATCH_INT: usize = 2;
const WATCH_RST: usize = 3;

const IMAGE_SDA: usize = 0;
const IMAGE_INT: usize = 1;

/// 7-bit address. A write byte is 0x2A and a read byte is 0x2B.
const ADDRESS: u8 = 0x15;

/// Register addresses 0x01 to 0x06 and 0xA7. The layout the open drivers read, with no
/// datasheet in hand.
const REG_GESTURE: usize = 0x01;
const REG_FINGERS: usize = 0x02;
const REG_X_HIGH: usize = 0x03;
const REG_X_LOW: usize = 0x04;
const REG_Y_HIGH: usize = 0x05;
const REG_Y_LOW: usize = 0x06;
const REG_CHIP: usize = 0xA7;

/// The byte those drivers expect at [`REG_CHIP`] for a CST816S. A guess, with no sheet in hand.
const CHIP_ID: u8 = 0xB4;

/// How long INT stays low after the 1 ps fall. A guess.
pub(crate) const INT_PULSE_PS: u64 = 100_000_000;

/// SDA and INT, released.
const RELEASED: [Drive; 2] = [Drive::Off, Drive::Off];

/// The pin pairs the chip joins at DC, for `net.floating`: SDA and INT can each pull to GND.
pub(crate) const DC_PATHS: [(usize, usize); 2] = [(SDA, GND), (INT_PIN, GND)];

pub(crate) fn chip(part: PartId, dev: &Cst816) -> Chip {
    let pins = dev.pins;
    let low = Stage {
        drop: dev.low_drop.si.get(),
        resistance: dev.low_resistance.si.get(),
    };
    let output = |pin| Output {
        pin,
        vcc: pins[VCC],
        gnd: pins[GND],
        high: low,
        low,
        pull_up: None,
        leakage: None,
    };
    let against = |pin: usize, fraction: f64| Signal {
        terms: vec![
            (Node::from(pins[pin]), 1.0),
            (Node::from(pins[VCC]), -fraction),
            (Node::from(pins[GND]), fraction - 1.0),
        ],
        offset: 0.0,
    };
    let watch = |pin: usize| Watch {
        rise: against(pin, dev.input_threshold.high.si.get()),
        fall: against(pin, dev.input_threshold.low.si.get()),
    };
    let mut signals = [
        (Cst816::PINS[SCL], WATCH_SCL),
        (Cst816::PINS[SDA], WATCH_SDA),
        (Cst816::PINS[INT_PIN], WATCH_INT),
        (Cst816::PINS[RST], WATCH_RST),
    ];
    signals.sort_unstable();
    Chip {
        part: Some(part),
        outputs: vec![output(pins[SDA]), output(pins[INT_PIN])],
        watches: vec![watch(SCL), watch(SDA), watch(INT_PIN), watch(RST)],
        signals: signals.to_vec(),
        samples: Vec::new(),
        usarts: &[],
        twi: false,
        sends: false,
        takes: crate::active::Takes::default(),
        core: crate::active::Core::Cst816(Box::new(Core::new())),
    }
}

/// The register file. A write transfer's first data byte sets `pointer`. Each later write byte,
/// and each read byte, uses `pointer` and then advances it by one, wrapping at 256. That is the
/// burst the open drivers read. The advance is inferred from that burst.
#[derive(Clone, Debug, PartialEq, Eq)]
struct File {
    mem: [u8; 256],
    pointer: u8,
    set_pointer: bool,
}

impl File {
    fn new() -> File {
        let mut file = File {
            mem: [0; 256],
            pointer: 0,
            set_pointer: false,
        };
        file.mem[REG_CHIP] = CHIP_ID;
        file
    }

    fn put(&mut self, touch: Touch) {
        self.mem[REG_GESTURE] = 0;
        self.mem[REG_FINGERS] = u8::from(touch.down);
        if !touch.down {
            return;
        }
        self.mem[REG_X_HIGH] = ((touch.x >> 8) as u8) & 0x0F;
        self.mem[REG_X_LOW] = touch.x as u8;
        self.mem[REG_Y_HIGH] = ((touch.y >> 8) as u8) & 0x0F;
        self.mem[REG_Y_LOW] = touch.y as u8;
    }
}

impl Target for File {
    fn matches(&self, address: u8) -> bool {
        address == ADDRESS
    }

    fn reset(&mut self) {
        *self = File::new();
    }

    fn start_write(&mut self) {
        self.set_pointer = true;
    }

    fn write(&mut self, _at: Time, byte: u8) -> bool {
        if self.set_pointer {
            self.pointer = byte;
            self.set_pointer = false;
            return true;
        }
        self.mem[usize::from(self.pointer)] = byte;
        self.pointer = self.pointer.wrapping_add(1);
        true
    }

    fn read(&mut self, _at: Time) -> u8 {
        let byte = self.mem[usize::from(self.pointer)];
        self.pointer = self.pointer.wrapping_add(1);
        byte
    }
}

/// Bus phase, register file, and the two open-drain drives. `now` is the time everything up to
/// here has been applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Core {
    engine: Engine,
    file: File,
    now: Time,
    drive: [Drive; 2],
    /// INT falls at this time.
    int_low: Option<Time>,
    /// INT releases at this time.
    int_high: Option<Time>,
}

impl Core {
    fn new() -> Core {
        Core {
            engine: Engine::new(),
            file: File::new(),
            now: Time::ZERO,
            drive: RELEASED,
            int_low: None,
            int_high: None,
        }
    }

    pub(crate) fn run_until(&mut self, limit: Time) -> Option<Stop> {
        assert!(limit >= self.now, "a core runs only past where it reached");
        let at = [self.engine.due_at(), self.int_low, self.int_high]
            .into_iter()
            .flatten()
            .min()
            .filter(|&at| at <= limit);
        let Some(at) = at else {
            self.now = limit;
            return None;
        };
        if self.engine.due_at() == Some(at)
            && let Some(low) = self.engine.take_due(at)
        {
            self.drive[IMAGE_SDA] = i2c::sda_drive(low);
        }
        if self.int_low == Some(at) {
            self.int_low = None;
            self.drive[IMAGE_INT] = Drive::Low;
        }
        if self.int_high == Some(at) {
            self.int_high = None;
            if self.int_low.is_none() {
                self.drive[IMAGE_INT] = Drive::Off;
            }
        }
        self.now = at;
        Some(Stop {
            at,
            sample: false,
            record: false,
        })
    }

    pub(crate) fn drive(&self) -> &[Drive] {
        &self.drive
    }

    pub(crate) fn sda_blocked_at(&self) -> Option<Time> {
        self.engine.sda_blocked_at()
    }

    pub(crate) fn input(&mut self, at: Time, levels: &[bool]) {
        self.catch_up(at);
        let entered = {
            let Core { engine, file, .. } = self;
            engine
                .edge(
                    at,
                    levels[WATCH_SCL],
                    levels[WATCH_SDA],
                    !levels[WATCH_RST],
                    file,
                )
                .entered_reset
        };
        if entered {
            self.release_int(at);
        }
    }

    /// The touch at `at`. The image stays as it is. INT falls at least 1 ps later.
    pub(crate) fn touch(&mut self, at: Time, touch: Touch) {
        self.catch_up(at);
        if self.engine.in_reset() {
            return;
        }
        let image = self.drive;
        self.file.put(touch);
        let fall = Time::from_ps(at.as_ps().saturating_add(1));
        let rise = Time::from_ps(fall.as_ps().saturating_add(INT_PULSE_PS));
        let already = self.drive[IMAGE_INT] == Drive::Low || self.int_low.is_some();
        if !already {
            self.int_low = Some(fall);
        }
        self.int_high = Some(match self.int_high {
            Some(old) if already => old.max(rise),
            _ => rise,
        });
        debug_assert_eq!(self.drive, image, "a touch never changes the image");
    }

    fn release_int(&mut self, at: Time) {
        self.int_low = None;
        if self.drive[IMAGE_INT] == Drive::Low || self.int_high.is_some() {
            self.int_high = at.as_ps().checked_add(1).map(Time::from_ps);
        }
    }

    fn catch_up(&mut self, at: Time) {
        if self.now < at {
            debug_assert!(
                [self.engine.due_at(), self.int_low, self.int_high]
                    .into_iter()
                    .flatten()
                    .all(|due| due > at),
                "a landing skipped a drive"
            );
            self.now = at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power_on() -> Core {
        let mut core = Core::new();
        core.input(Time::ZERO, &[true, true, true, true]);
        core
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
                self.core
                    .input(Time::from_ps(self.t), &[self.scl, sda, true, self.rst_high]);
                self.mark();
                let limit = self.t.saturating_add(crate::active::i2c::DATA_HOLD_PS + 2);
                while let Some(stop) = self.core.run_until(Time::from_ps(limit)) {
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
            self.core
                .input(Time::from_ps(t), &[scl, sda, true, self.rst_high]);
            self.mark();
        }

        fn spin(&mut self, limit: u64) {
            while let Some(stop) = self.core.run_until(Time::from_ps(limit)) {
                self.t = stop.at.as_ps();
                self.mark();
                let sda = self.wire_sda();
                self.core
                    .input(Time::from_ps(self.t), &[self.scl, sda, true, self.rst_high]);
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
        bus.core.touch(
            Time::from_ps(500),
            Touch {
                x: 0x123,
                y: 0x045,
                down: true,
            },
        );
        bus.core.run_until(Time::from_ps(501));
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
        core.touch(
            at,
            Touch {
                x: 10,
                y: 20,
                down: true,
            },
        );
        assert_eq!(core.drive(), &RELEASED);
        assert_eq!(core.run_until(at), None, "no drive at the touch");
        let fall = Time::from_ps(at.as_ps() + 1);
        let stop = core.run_until(fall).expect("INT falls");
        assert_eq!(stop.at, fall);
        assert_eq!(core.drive()[IMAGE_INT], Drive::Low);
        let rise = Time::from_ps(fall.as_ps() + INT_PULSE_PS);
        assert_eq!(core.run_until(Time::from_ps(rise.as_ps() - 1)), None);
        let stop = core.run_until(rise).expect("INT rises");
        assert_eq!(stop.at, rise);
        assert_eq!(core.drive()[IMAGE_INT], Drive::Off);
    }

    #[test]
    fn reset_releases_int_and_clears_a_point() {
        let mut core = power_on();
        core.touch(
            Time::from_ps(1_000),
            Touch {
                x: 1,
                y: 2,
                down: true,
            },
        );
        let fall = Time::from_ps(1_001);
        core.run_until(fall);
        assert_eq!(core.drive()[IMAGE_INT], Drive::Low);
        core.input(Time::from_ps(2_000), &[true, true, false, false]);
        let release = Time::from_ps(2_001);
        let stop = core.run_until(release).expect("INT releases");
        assert_eq!(stop.at, release);
        assert_eq!(core.drive(), &RELEASED);
        assert_eq!(core.file.mem[REG_FINGERS], 0);
        assert_eq!(core.file.mem[REG_CHIP], CHIP_ID);
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
        bus.core.touch(
            Time::from_ps(500),
            Touch {
                x: 291,
                y: 69,
                down: true,
            },
        );
        bus.core.run_until(Time::from_ps(501));
        bus.core.touch(
            Time::from_ps(3_000),
            Touch {
                x: 0,
                y: 0,
                down: false,
            },
        );
        bus.core.run_until(Time::from_ps(3_001));
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
        bus.core.touch(
            Time::from_ps(1_000),
            Touch {
                x: 4,
                y: 5,
                down: true,
            },
        );
        bus.core.run_until(Time::from_ps(1_001));
        assert_eq!(bus.core.drive()[IMAGE_INT], Drive::Low);
        bus.t = 1_001;
        bus.start();
        bus.byte(ADDRESS << 1);
        let fall = bus.t + 1_000;
        bus.set(fall, false, true);
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS);
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
        bus.spin(rst_at + crate::active::i2c::DATA_HOLD_PS);
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
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS + 2);
        assert!(!bus.sda_low(), "a 100 ns low leaves the ACK undriven");
        let first = bus.core.sda_blocked_at().expect("the rule is recorded");
        assert_eq!(first.as_ps(), fall + crate::active::i2c::DATA_HOLD_PS);
        bus.stop();
        bus.start();
        bus.byte(ADDRESS << 1);
        let fall = bus.t + 1_000;
        bus.set(fall, false, true);
        bus.set(fall + 100_000, true, true);
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS + 2);
        assert_eq!(
            bus.core.sda_blocked_at(),
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
        assert_eq!(bus.core.sda_blocked_at(), None);
    }

    #[test]
    fn reset_while_scl_is_high_releases_sda_and_logs_nothing() {
        let mut bus = Bus::new();
        bus.core.touch(
            Time::from_ps(1_000),
            Touch {
                x: 4,
                y: 5,
                down: true,
            },
        );
        bus.core.run_until(Time::from_ps(1_001));
        assert_eq!(bus.core.drive()[IMAGE_INT], Drive::Low);
        bus.t = 1_001;
        bus.start();
        bus.byte(ADDRESS << 1);
        let fall = bus.t + 1_000;
        bus.set(fall, false, true);
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS);
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
        bus.spin(rst_at + crate::active::i2c::DATA_HOLD_PS);
        assert!(!bus.sda_low(), "SDA releases while SCL is high");
        assert_eq!(bus.core.sda_blocked_at(), None, "reset logs no warning");
    }

    fn lets_go_and_stops(bus: &mut Bus) {
        let fall = bus.t + 1_000;
        bus.set(fall, false, true);
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS + 2);
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
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS + 2);
        assert!(bus.sda_low(), "the release is still the target's");
        let blocked = bus.core.sda_blocked_at().expect("the rule is recorded");
        assert_eq!(blocked.as_ps(), fall + crate::active::i2c::DATA_HOLD_PS);
        lets_go_and_stops(&mut bus);
        assert_eq!(bus.core.sda_blocked_at(), Some(blocked));
    }

    #[test]
    fn a_refused_address_ack_does_not_read_on() {
        let mut bus = Bus::new();
        bus.fast_start();
        bus.fast_byte((ADDRESS << 1) | 1);
        let fall = bus.t + 1_000;
        bus.set(fall, false, true);
        bus.set(fall + 100_000, true, true);
        bus.spin(fall + crate::active::i2c::DATA_HOLD_PS + 2);
        assert!(!bus.sda_low(), "the ACK stays undriven");
        assert!(bus.core.sda_blocked_at().is_some(), "the rule is recorded");
        lets_go_and_stops(&mut bus);
    }
}

/// A CST816 touch controller, an active chip with no clock of its own. It is an I2C target at
/// address 0x15. `SDA` and `INT` are open drain into `GND` through `low_resistance` above
/// `low_drop`. `SCL` is an input, and the part does not hold it. `RST` low clears it. It reads
/// each of those four pins through `input_threshold`, fractions of its supply.
///
/// `touches` is the finger. Each event is one point, after t = 0 and at least [`MIN_GAP_PS`]
/// after the previous event. `x` and `y` run from 0 to [`Cst816::COORD_MAX`].
#[derive(Clone, Debug, PartialEq)]
pub struct Cst816 {
    pub pins: [NetId; 6],
    /// Volt, at least 0. The open-drain low side.
    pub low_drop: Value,
    /// Ohm.
    pub low_resistance: Value,
    pub input_threshold: Threshold,
    pub touches: Vec<Event<Touch>>,
}

/// One point of a [`Cst816`]: the finger is down at `(x, y)`, or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub x: u16,
    pub y: u16,
    pub down: bool,
}

impl Cst816 {
    pub const PINS: [&'static str; 6] = ["VCC", "GND", "SCL", "SDA", "INT", "RST"];
    /// The largest `x` or `y`.
    pub const COORD_MAX: u16 = 4095;

    /// The next touch strictly after `at`.
    pub fn next_touch_after(&self, at: Time) -> Option<Time> {
        let k = self.touches.partition_point(|event| event.at <= at);
        self.touches.get(k).map(|event| event.at)
    }

    /// The first time at or after `from` that one more touch passes the schedule rules.
    /// With no touch that is 1 ps. After one it is that touch plus [`MIN_GAP_PS`].
    pub fn earliest(&self, from: Time) -> Option<Time> {
        let first = match self.touches.last() {
            None => 1,
            Some(last) => last.at.as_ps().checked_add(MIN_GAP_PS)?,
        };
        Some(Time::from_ps(from.as_ps().max(first)))
    }
}

// The `cst816` kind: its pins, its open-drain low side, its input threshold and its touches.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diagnostic::{Diagnostic, ScheduleRule, ValueRule};
use crate::name::{Name, Ref};
use crate::netlist::json::threshold::{WireThreshold, resolve_threshold};
use crate::netlist::json::{PinMap, WireValue, resolve_pins, resolve_value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireCst816 {
    pub(crate) name: String,
    pins: PinMap,
    low_drop: WireValue,
    low_resistance: WireValue,
    input_threshold: WireThreshold,
    #[serde(default)]
    touches: Vec<WireTouch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTouch {
    t_ps: u64,
    x: u16,
    y: u16,
    down: bool,
}

pub(crate) fn resolve_cst816(
    name: &Name,
    part_ref: &Ref,
    wire: WireCst816,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Cst816> {
    let pins = resolve_pins(name, Cst816::PINS, &wire.pins.0, nets, out);
    let low_drop = resolve_drop(part_ref, wire.low_drop, out);
    let low_resistance = resolve_value(
        part_ref,
        "low_resistance",
        "ohm",
        true,
        wire.low_resistance,
        out,
    );
    let input_threshold = resolve_threshold(
        part_ref,
        ["input_threshold.low", "input_threshold.high"],
        wire.input_threshold,
        out,
    );
    let touches = resolve_touches(part_ref, wire.touches, out);
    Some(Cst816 {
        pins: pins?,
        low_drop: low_drop?,
        low_resistance: low_resistance?,
        input_threshold: input_threshold?,
        touches: touches?,
    })
}

fn resolve_drop(
    part: &Ref,
    wire: WireValue,
    out: &mut Vec<Diagnostic>,
) -> Option<crate::netlist::Value> {
    let volts = wire.value.get();
    let value = resolve_value(part, "low_drop", "V", false, wire, out)?;
    if volts < 0.0 {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "low_drop".into(),
            value: volts,
            rule: ValueRule::NonNegative,
        });
        return None;
    }
    Some(value)
}

fn resolve_touches(
    part: &Ref,
    wires: Vec<WireTouch>,
    out: &mut Vec<Diagnostic>,
) -> Option<Vec<Event<Touch>>> {
    let mut ok = true;
    let mut previous: Option<u64> = None;
    let mut earliest = Some(0u64);
    let mut events = Vec::with_capacity(wires.len());
    for (index, wire) in wires.into_iter().enumerate() {
        let at = wire.t_ps;
        let rule = if previous.is_some_and(|previous| at <= previous) {
            Some(ScheduleRule::NotIncreasing)
        } else if at == 0 {
            Some(ScheduleRule::AtZero)
        } else if earliest.is_none_or(|earliest| at < earliest) {
            Some(ScheduleRule::TooClose)
        } else {
            None
        };
        if let Some(rule) = rule {
            ok = false;
            out.push(Diagnostic::NetlistSchedule {
                part: part.clone(),
                field: "touches",
                event: index,
                rule,
                end_ps: 0,
            });
        }
        for (axis, value) in [("x", wire.x), ("y", wire.y)] {
            if value > Cst816::COORD_MAX {
                ok = false;
                out.push(Diagnostic::NetlistValue {
                    part: part.clone(),
                    field: format!("touches[{index}].{axis}").into(),
                    value: f64::from(value),
                    rule: ValueRule::Span,
                });
            }
        }
        events.push(Event {
            at: Time::from_ps(at),
            to: Touch {
                x: wire.x,
                y: wire.y,
                down: wire.down,
            },
        });
        previous = Some(at);
        earliest = at.checked_add(MIN_GAP_PS);
    }
    ok.then_some(events)
}
