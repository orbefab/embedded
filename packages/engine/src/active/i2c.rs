//! The host's I2C target engine on SCL and SDA. It matches the 7-bit address its device
//! declares, drives SDA low for an ACK and for a zero bit of a read, and releases SDA otherwise.
//! It never drives SCL. The device sees transfers and bytes, through [`Target`].
//!
//! A bit is sampled on the target's own SCL rise. SDA moves [`DATA_HOLD_PS`] after the
//! target's own SCL fall. The image is unchanged at the input.

/// 300 ns, this model's choice. UM10204 Rev. 7.0 Table 11 gives `tHD;DAT` a minimum of 0 for
/// an I2C device. Note [2] on that row says SCL must cross 0.3 VDD before SDA changes. The
/// 300 ns figure in that table is `tf`. `tSU;DAT` is 250 ns, 100 ns and 50 ns. Fast-mode Plus
/// `tVD;DAT` max is 0.45 us. This hold clears the fall note with margin and stays inside
/// `tVD;DAT` for every mode. The target needs SCL low for more than this.
pub(crate) const DATA_HOLD_PS: u64 = 300_000;

use orbe_model::{Ack, Dir, Drive, I2cTarget, Time};

use super::native::{Hosting, Port};
use crate::diagnostic::ProtocolRule;

/// The bus a device declares: its address, and which of its watches are SCL, SDA and the pin
/// that holds the bus in reset while it reads low.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bus {
    pub(crate) address: u8,
    pub(crate) scl: usize,
    pub(crate) sda: usize,
    pub(crate) reset: Option<usize>,
}

/// What the engine calls for the bytes: a device's [`I2cTarget`], or a test's recorder.
pub(crate) trait Target {
    /// The address matched, to write to the target or to read from it.
    fn start(&mut self, at: Time, dir: Dir);
    /// The eighth SCL rise of a write delivered `byte`.
    fn write(&mut self, at: Time, byte: u8) -> Ack;
    /// The SCL fall after an address ACK, or after a master ACK that asked for another byte.
    fn read(&mut self, at: Time) -> u8;
    /// The master's ACK or NACK rise during a read. `more` is true when the master asked for
    /// another byte. [`Target::read`] of that byte runs on the following fall.
    fn master_ack(&mut self, at: Time, more: bool);
    /// A STOP, from any phase.
    fn stop(&mut self, at: Time);
}

impl<D: I2cTarget> Target for Hosting<D> {
    fn start(&mut self, at: Time, dir: Dir) {
        self.call(at, |device, cx| device.start(cx, at, dir));
    }

    fn write(&mut self, at: Time, byte: u8) -> Ack {
        self.call(at, |device, cx| device.write(cx, at, byte))
    }

    fn read(&mut self, at: Time) -> u8 {
        self.call(at, |device, cx| device.read(cx, at))
    }

    fn master_ack(&mut self, at: Time, more: bool) {
        self.call(at, |device, cx| device.master_ack(cx, at, more));
    }

    fn stop(&mut self, at: Time) {
        self.call(at, |device, cx| device.stop(cx, at));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recv {
    Address,
    Data,
}

/// What follows the address ACK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Write,
    Read,
    Idle,
}

/// `left` counts bits still to sample. `bit` is the bit index currently on the wire, 7 first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Recv {
        kind: Recv,
        shifter: u8,
        left: u8,
    },
    /// The byte is in. The next SCL fall drives the ACK.
    DriveAck {
        ack: bool,
        next: Next,
    },
    /// The ACK level is on the wire. The next SCL fall ends it.
    HoldAck {
        next: Next,
    },
    /// `bit` of `byte` is on the wire. The next SCL fall ends it.
    HoldBit {
        byte: u8,
        bit: u8,
    },
    /// SDA is released. The next SCL rise is the master's ACK.
    SampleAck,
    /// The master's ACK has been sampled. The next SCL fall starts another byte or stops.
    AfterAck {
        cont: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Level { at: Time, low: bool },
    ResetRelease { at: Time },
}

impl Pending {
    fn at(self) -> Time {
        match self {
            Pending::Level { at, .. } | Pending::ResetRelease { at } => at,
        }
    }

    fn low(self) -> bool {
        match self {
            Pending::Level { low, .. } => low,
            Pending::ResetRelease { .. } => false,
        }
    }

    fn forced(self) -> bool {
        matches!(self, Pending::ResetRelease { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Engine {
    bus: Bus,
    phase: Phase,
    seen: bool,
    in_reset: bool,
    scl: bool,
    sda: bool,
    /// Whether SDA is driven low now.
    sda_low: bool,
    /// Drive SDA low, or release it, at this time.
    due: Option<Pending>,
    sda_blocked_at: Option<Time>,
    release_on_fall: bool,
}

impl Engine {
    pub(crate) const fn new(bus: Bus) -> Engine {
        Engine {
            bus,
            phase: Phase::Idle,
            seen: false,
            in_reset: false,
            scl: true,
            sda: true,
            sda_low: false,
            due: None,
            sda_blocked_at: None,
            release_on_fall: false,
        }
    }

    pub(crate) fn due_at(self) -> Option<Time> {
        self.due.map(Pending::at)
    }

    /// Applies the drive due at `at` and reports whether SDA is driven low afterwards.
    pub(crate) fn take_due(&mut self, at: Time) -> Option<bool> {
        let pending = self.due?;
        if pending.at() != at {
            return None;
        }
        self.due = None;
        let low = pending.low();
        if !pending.forced() && self.scl && low != self.sda_low {
            if self.sda_blocked_at.is_none() {
                self.sda_blocked_at = Some(at);
            }
            self.phase = Phase::Idle;
            if self.sda_low {
                self.release_on_fall = true;
            }
            return None;
        }
        self.sda_low = low;
        Some(low)
    }

    pub(crate) fn edge(
        &mut self,
        at: Time,
        scl: bool,
        sda: bool,
        reset: bool,
        dev: &mut impl Target,
    ) {
        if reset {
            if !self.in_reset {
                self.phase = Phase::Idle;
                self.release_on_fall = false;
                self.release_anyway(at);
            }
            self.in_reset = true;
            self.note(scl, sda);
            return;
        }
        let left_reset = self.in_reset;
        self.in_reset = false;
        if !self.seen || left_reset {
            self.note(scl, sda);
            return;
        }
        let scl_rose = scl && !self.scl;
        let scl_fell = !scl && self.scl;
        let sda_fell = !sda && self.sda;
        let sda_rose = sda && !self.sda;
        self.note(scl, sda);
        if self.scl && sda_fell {
            self.phase = Phase::Recv {
                kind: Recv::Address,
                shifter: 0,
                left: 8,
            };
            self.want(at, false);
        } else if self.scl && sda_rose {
            self.phase = Phase::Idle;
            self.want(at, false);
            dev.stop(at);
        } else if scl_rose {
            self.on_rise(at, dev);
        } else if scl_fell {
            self.on_fall(at, dev);
        }
    }

    fn note(&mut self, scl: bool, sda: bool) {
        self.seen = true;
        self.scl = scl;
        self.sda = sda;
    }

    fn on_rise(&mut self, at: Time, dev: &mut impl Target) {
        match self.phase {
            Phase::Recv {
                kind,
                shifter,
                left,
            } => {
                let shifter = (shifter << 1) | u8::from(self.sda);
                let left = left - 1;
                if left > 0 {
                    self.phase = Phase::Recv {
                        kind,
                        shifter,
                        left,
                    };
                    return;
                }
                self.phase = match kind {
                    Recv::Address => {
                        let address = shifter >> 1;
                        let read = shifter & 1 == 1;
                        let matched = address == self.bus.address;
                        let next = if !matched {
                            Next::Idle
                        } else if read {
                            dev.start(at, Dir::Read);
                            Next::Read
                        } else {
                            dev.start(at, Dir::Write);
                            Next::Write
                        };
                        Phase::DriveAck { ack: matched, next }
                    }
                    Recv::Data => {
                        let ack = dev.write(at, shifter) == Ack::Yes;
                        Phase::DriveAck {
                            ack,
                            next: if ack { Next::Write } else { Next::Idle },
                        }
                    }
                };
            }
            Phase::SampleAck => {
                if self.sda_low {
                    self.phase = Phase::Idle;
                } else {
                    let ack = !self.sda;
                    self.phase = Phase::AfterAck { cont: ack };
                    dev.master_ack(at, ack);
                }
            }
            Phase::Idle
            | Phase::DriveAck { .. }
            | Phase::HoldAck { .. }
            | Phase::HoldBit { .. }
            | Phase::AfterAck { .. } => {}
        }
    }

    fn on_fall(&mut self, at: Time, dev: &mut impl Target) {
        if self.release_on_fall {
            self.release_on_fall = false;
            self.want(at, false);
        }
        match self.phase {
            Phase::DriveAck { ack, next } => {
                self.want(at, ack);
                self.phase = Phase::HoldAck { next };
            }
            Phase::HoldAck { next, .. } => self.begin(at, next, dev),
            Phase::HoldBit { byte, bit } if bit > 0 => {
                let bit = bit - 1;
                self.drive_bit(at, byte, bit);
                self.phase = Phase::HoldBit { byte, bit };
            }
            Phase::HoldBit { .. } => {
                self.want(at, false);
                self.phase = Phase::SampleAck;
            }
            Phase::AfterAck { cont: true } => self.begin(at, Next::Read, dev),
            Phase::AfterAck { cont: false } => {
                self.want(at, false);
                self.phase = Phase::Idle;
            }
            Phase::Idle | Phase::Recv { .. } | Phase::SampleAck => {}
        }
    }

    fn begin(&mut self, at: Time, next: Next, dev: &mut impl Target) {
        match next {
            Next::Write => {
                self.want(at, false);
                self.phase = Phase::Recv {
                    kind: Recv::Data,
                    shifter: 0,
                    left: 8,
                };
            }
            Next::Read => {
                let byte = dev.read(at);
                self.drive_bit(at, byte, 7);
                self.phase = Phase::HoldBit { byte, bit: 7 };
            }
            Next::Idle => {
                self.want(at, false);
                self.phase = Phase::Idle;
            }
        }
    }

    fn drive_bit(&mut self, at: Time, byte: u8, bit: u8) {
        self.want(at, (byte >> bit) & 1 == 0);
    }

    fn want(&mut self, at: Time, low: bool) {
        let effective = self.due.map(Pending::low).unwrap_or(self.sda_low);
        if effective == low {
            return;
        }
        let Some(at) = at.as_ps().checked_add(DATA_HOLD_PS) else {
            return;
        };
        self.due = Some(Pending::Level {
            at: Time::from_ps(at),
            low,
        });
    }

    fn release_anyway(&mut self, at: Time) {
        if let Some(pending) = self.due {
            if !pending.low() {
                self.due = Some(Pending::ResetRelease { at: pending.at() });
                return;
            }
        } else if !self.sda_low {
            return;
        }
        let Some(at) = at.as_ps().checked_add(DATA_HOLD_PS) else {
            return;
        };
        self.due = Some(Pending::ResetRelease {
            at: Time::from_ps(at),
        });
    }
}

/// SDA, the one pin the engine drives.
impl<D: I2cTarget> Port<D> for Engine {
    const PINS: usize = 1;

    fn drive(&self, image: &mut [Drive]) {
        image[0] = if self.sda_low { Drive::Low } else { Drive::Off };
    }

    fn due_at(&self) -> Option<Time> {
        Engine::due_at(*self)
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, at: Time) {
        Engine::take_due(self, at);
    }

    fn input(&mut self, dev: &mut Hosting<D>, at: Time, levels: &[bool]) {
        let Bus {
            scl, sda, reset, ..
        } = self.bus;
        let reset = reset.is_some_and(|pin| !levels[pin]);
        self.edge(at, levels[scl], levels[sda], reset, dev);
    }

    fn broke(&self, found: &mut Vec<(ProtocolRule, Time)>) {
        found.extend(
            self.sda_blocked_at
                .map(|at| (ProtocolRule::SdaWhileSclHigh, at)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Ev {
        StartWrite,
        StartRead,
        Write(u8),
        Read(u8),
        MasterAck(bool),
        Stop,
    }

    struct Dev {
        events: Vec<Ev>,
        mem: [u8; 256],
        pointer: u8,
        set_pointer: bool,
        nack: Option<u8>,
    }

    impl Dev {
        fn new() -> Dev {
            let mut dev = Dev {
                events: Vec::new(),
                mem: [0; 256],
                pointer: 0,
                set_pointer: false,
                nack: None,
            };
            dev.mem[0xA7] = 0xB4;
            dev
        }
    }

    impl Target for Dev {
        fn start(&mut self, _at: Time, dir: Dir) {
            match dir {
                Dir::Write => {
                    self.events.push(Ev::StartWrite);
                    self.set_pointer = true;
                }
                Dir::Read => self.events.push(Ev::StartRead),
            }
        }

        fn write(&mut self, _at: Time, byte: u8) -> Ack {
            self.events.push(Ev::Write(byte));
            if self.set_pointer {
                self.pointer = byte;
                self.set_pointer = false;
                return Ack::Yes;
            }
            if self.nack == Some(self.pointer) {
                return Ack::No;
            }
            self.mem[usize::from(self.pointer)] = byte;
            self.pointer = self.pointer.wrapping_add(1);
            Ack::Yes
        }

        fn read(&mut self, _at: Time) -> u8 {
            let byte = self.mem[usize::from(self.pointer)];
            self.pointer = self.pointer.wrapping_add(1);
            self.events.push(Ev::Read(byte));
            byte
        }

        fn master_ack(&mut self, _at: Time, more: bool) {
            self.events.push(Ev::MasterAck(more));
        }

        fn stop(&mut self, _at: Time) {
            self.events.push(Ev::Stop);
        }
    }

    struct Step {
        engine: Engine,
        t: u64,
        scl: bool,
        master: bool,
        driven: bool,
    }

    impl Step {
        fn power(dev: &mut Dev) -> Step {
            let mut step = Step {
                engine: Engine::new(Bus {
                    address: 0x15,
                    scl: 0,
                    sda: 1,
                    reset: None,
                }),
                t: 0,
                scl: true,
                master: true,
                driven: false,
            };
            step.go(dev, true, true);
            step
        }

        fn wire(&self) -> bool {
            self.master && !self.driven
        }

        fn go(&mut self, dev: &mut Dev, scl: bool, master: bool) {
            self.t += 1_000;
            self.scl = scl;
            self.master = master;
            self.engine
                .edge(Time::from_ps(self.t), scl, self.wire(), false, dev);
            self.pump(dev);
        }

        fn pump(&mut self, dev: &mut Dev) {
            let limit = self.t + DATA_HOLD_PS + 2;
            while let Some(due) = self.engine.due_at() {
                if due.as_ps() > limit {
                    break;
                }
                match self.engine.take_due(due) {
                    Some(low) => {
                        self.driven = low;
                        self.t = due.as_ps();
                        self.engine.edge(due, self.scl, self.wire(), false, dev);
                    }
                    None => break,
                }
            }
        }

        fn start(&mut self, dev: &mut Dev) {
            self.go(dev, true, true);
            self.go(dev, true, false);
        }

        fn bit(&mut self, dev: &mut Dev, high: bool) {
            self.go(dev, false, high);
            self.go(dev, true, high);
        }

        fn byte(&mut self, dev: &mut Dev, byte: u8) {
            for shift in (0..8).rev() {
                self.bit(dev, (byte >> shift) & 1 == 1);
            }
        }

        fn ack_clock(&mut self, dev: &mut Dev) {
            self.go(dev, false, true);
            self.go(dev, true, true);
        }

        fn read_byte(&mut self, dev: &mut Dev) {
            for _ in 0..8 {
                self.go(dev, false, true);
                self.go(dev, true, true);
            }
        }

        fn master_ack(&mut self, dev: &mut Dev, ack: bool) {
            self.go(dev, false, !ack);
            self.go(dev, true, !ack);
        }

        fn ack_rise_while_still_driving(&mut self, dev: &mut Dev) {
            self.t += 1_000;
            self.scl = false;
            self.master = true;
            self.engine
                .edge(Time::from_ps(self.t), false, self.wire(), false, dev);
            self.t += 1_000;
            self.scl = true;
            self.engine
                .edge(Time::from_ps(self.t), true, self.wire(), false, dev);
        }

        fn stop(&mut self, dev: &mut Dev) {
            self.go(dev, false, false);
            self.go(dev, true, false);
            self.go(dev, true, true);
        }

        fn repeated_start(&mut self, dev: &mut Dev) {
            self.go(dev, false, true);
            self.go(dev, true, true);
            self.go(dev, true, false);
        }
    }

    #[test]
    fn another_address_starts_no_transfer() {
        let mut dev = Dev::new();
        let mut step = Step::power(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, 0x16 << 1);
        step.ack_clock(&mut dev);
        step.stop(&mut dev);
        assert_eq!(dev.events, vec![Ev::Stop]);
    }

    #[test]
    fn a_refused_byte_is_nacked_and_a_master_ack_precedes_the_next_read() {
        let mut dev = Dev::new();
        dev.nack = Some(0x11);
        let mut step = Step::power(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, 0x15 << 1);
        step.ack_clock(&mut dev);
        assert!(step.driven, "the address is ACKed");
        step.byte(&mut dev, 0x10);
        step.ack_clock(&mut dev);
        assert!(step.driven, "0x10 is ACKed");
        step.byte(&mut dev, 0x5A);
        step.ack_clock(&mut dev);
        assert!(step.driven, "0x5A is ACKed");
        step.byte(&mut dev, 0x5B);
        step.ack_clock(&mut dev);
        assert!(!step.driven, "0x5B, at the refused pointer, is NACKed");
        step.stop(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, 0x15 << 1);
        step.ack_clock(&mut dev);
        step.byte(&mut dev, 0xA7);
        step.ack_clock(&mut dev);
        assert!(step.driven, "0xA7 is ACKed");
        step.repeated_start(&mut dev);
        step.byte(&mut dev, (0x15 << 1) | 1);
        step.ack_clock(&mut dev);
        assert!(step.driven, "the read address is ACKed");
        step.read_byte(&mut dev);
        step.master_ack(&mut dev, true);
        step.read_byte(&mut dev);
        step.master_ack(&mut dev, false);
        step.stop(&mut dev);
        assert_eq!(
            dev.events,
            vec![
                Ev::StartWrite,
                Ev::Write(0x10),
                Ev::Write(0x5A),
                Ev::Write(0x5B),
                Ev::Stop,
                Ev::StartWrite,
                Ev::Write(0xA7),
                Ev::StartRead,
                Ev::Read(0xB4),
                Ev::MasterAck(true),
                Ev::Read(0),
                Ev::MasterAck(false),
                Ev::Stop,
            ]
        );
    }

    /// Bit 0 of 0xB4 is 0, so the target still drives SDA when the ACK rise comes inside the
    /// hold. The events stop at that read. After the next fall the target is not driving, and
    /// no read ACK or further byte appears.
    #[test]
    fn a_read_ack_while_the_target_still_drives_goes_idle() {
        let mut dev = Dev::new();
        dev.mem[0] = 0xB4;
        let mut step = Step::power(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, (0x15 << 1) | 1);
        step.ack_clock(&mut dev);
        step.read_byte(&mut dev);
        assert!(step.driven, "bit 0 of 0xB4 holds SDA");
        step.ack_rise_while_still_driving(&mut dev);
        let held = vec![Ev::StartRead, Ev::Read(0xB4)];
        assert_eq!(dev.events, held);
        step.t += 1_000;
        step.scl = false;
        step.engine
            .edge(Time::from_ps(step.t), false, step.wire(), false, &mut dev);
        step.pump(&mut dev);
        assert!(!step.driven, "the target is not driving after the fall");
        assert_eq!(dev.events, held, "no read ack and no further byte");
    }
}
