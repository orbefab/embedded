//! An I2C target on SCL and SDA. It matches a 7-bit address, drives SDA low for an ACK and for
//! a zero bit of a read, and releases SDA otherwise. It never drives SCL.
//!
//! A bit is sampled on the target's own SCL rise. SDA moves [`DATA_HOLD_PS`] after the
//! target's own SCL fall. The image is unchanged at the input.

/// 300 ns, this model's choice. UM10204 Rev. 7.0 Table 11 gives `tHD;DAT` a minimum of 0 for
/// an I2C device. Note [2] on that row says SCL must cross 0.3 VDD before SDA changes. The
/// 300 ns figure in that table is `tf`. `tSU;DAT` is 250 ns, 100 ns and 50 ns. Fast-mode Plus
/// `tVD;DAT` max is 0.45 us. This hold clears the fall note with margin and stays inside
/// `tVD;DAT` for every mode. The target needs SCL low for more than this.
pub(crate) const DATA_HOLD_PS: u64 = 300_000;

use orbe_engine_core::time::Time;

use super::Drive;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AckKind {
    Address,
    Byte,
}

/// What the target stores. The engine owns the bus phases and calls this for the bytes.
pub(crate) trait Target {
    fn matches(&self, address: u8) -> bool;
    fn reset(&mut self);
    /// The address matched as a write. What the following [`Target::write`] bytes mean is the
    /// target's own, such as a register pointer and then data.
    fn start_write(&mut self) {}
    fn start_read(&mut self) {}
    /// The eighth SCL rise of a write delivered `byte`. Return whether to ACK it.
    fn write(&mut self, at: Time, byte: u8) -> bool;
    /// The SCL fall after an address ACK, or after a master ACK that asked for another byte.
    fn read(&mut self, at: Time) -> u8;
    /// This target's own ACK rise. `kind` is the address or the data byte, and `accepted` is
    /// whether this target drove the ACK. Another device's address does not call this.
    fn on_ack(&mut self, _at: Time, _kind: AckKind, _accepted: bool) {}
    /// The master's ACK or NACK rise during a read. `ack` is true when the master asked for
    /// another byte. A target that samples its pins for the next byte does it here.
    /// [`Target::read`] of that byte runs on the following fall.
    fn on_read_ack(&mut self, _at: Time, _ack: bool) {}
    /// A STOP, from any phase.
    fn on_stop(&mut self, _at: Time) {}
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
        address: bool,
    },
    /// The ACK level is on the wire. The next SCL fall ends it.
    HoldAck {
        next: Next,
        kind: AckKind,
        accepted: bool,
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

pub(crate) struct Edge {
    pub(crate) entered_reset: bool,
}

impl Engine {
    pub(crate) const fn new() -> Engine {
        Engine {
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

    pub(crate) fn sda_blocked_at(self) -> Option<Time> {
        self.sda_blocked_at
    }

    pub(crate) fn in_reset(self) -> bool {
        self.in_reset
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
    ) -> Edge {
        if reset {
            let entered = !self.in_reset;
            self.in_reset = true;
            if entered {
                self.phase = Phase::Idle;
                self.release_on_fall = false;
                self.release_anyway(at);
                dev.reset();
            }
            self.note(scl, sda);
            return Edge {
                entered_reset: entered,
            };
        }
        let left_reset = self.in_reset;
        self.in_reset = false;
        if !self.seen || left_reset {
            self.note(scl, sda);
            return Edge {
                entered_reset: false,
            };
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
            dev.on_stop(at);
        } else if scl_rose {
            self.on_rise(at, dev);
        } else if scl_fell {
            self.on_fall(at, dev);
        }
        Edge {
            entered_reset: false,
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
                        let matched = dev.matches(address);
                        let next = if !matched {
                            Next::Idle
                        } else if read {
                            dev.start_read();
                            Next::Read
                        } else {
                            dev.start_write();
                            Next::Write
                        };
                        Phase::DriveAck {
                            ack: matched,
                            next,
                            address: true,
                        }
                    }
                    Recv::Data => {
                        let ack = dev.write(at, shifter);
                        Phase::DriveAck {
                            ack,
                            next: if ack { Next::Write } else { Next::Idle },
                            address: false,
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
                    dev.on_read_ack(at, ack);
                }
            }
            Phase::HoldAck { kind, accepted, .. } => {
                let on_wire = if accepted {
                    self.sda_low
                } else {
                    !self.sda_low
                };
                if on_wire && !(kind == AckKind::Address && !accepted) {
                    dev.on_ack(at, kind, accepted);
                }
            }
            Phase::Idle
            | Phase::DriveAck { .. }
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
            Phase::DriveAck { ack, next, address } => {
                self.want(at, ack);
                self.phase = Phase::HoldAck {
                    next,
                    kind: if address {
                        AckKind::Address
                    } else {
                        AckKind::Byte
                    },
                    accepted: ack,
                };
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

pub(crate) fn sda_drive(low: bool) -> Drive {
    if low { Drive::Low } else { Drive::Off }
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
        Ack(AckKind, bool),
        ReadAck(bool),
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
        fn matches(&self, address: u8) -> bool {
            address == 0x15
        }

        fn reset(&mut self) {}

        fn start_write(&mut self) {
            self.events.push(Ev::StartWrite);
            self.set_pointer = true;
        }

        fn start_read(&mut self) {
            self.events.push(Ev::StartRead);
        }

        fn write(&mut self, _at: Time, byte: u8) -> bool {
            self.events.push(Ev::Write(byte));
            if self.set_pointer {
                self.pointer = byte;
                self.set_pointer = false;
                return true;
            }
            if self.nack == Some(self.pointer) {
                return false;
            }
            self.mem[usize::from(self.pointer)] = byte;
            self.pointer = self.pointer.wrapping_add(1);
            true
        }

        fn read(&mut self, _at: Time) -> u8 {
            let byte = self.mem[usize::from(self.pointer)];
            self.pointer = self.pointer.wrapping_add(1);
            self.events.push(Ev::Read(byte));
            byte
        }

        fn on_ack(&mut self, _at: Time, kind: AckKind, accepted: bool) {
            self.events.push(Ev::Ack(kind, accepted));
        }

        fn on_read_ack(&mut self, _at: Time, ack: bool) {
            self.events.push(Ev::ReadAck(ack));
        }

        fn on_stop(&mut self, _at: Time) {
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
                engine: Engine::new(),
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
    fn another_address_does_not_call_on_ack() {
        let mut dev = Dev::new();
        let mut step = Step::power(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, 0x16 << 1);
        step.ack_clock(&mut dev);
        step.stop(&mut dev);
        assert_eq!(dev.events, vec![Ev::Stop]);
    }

    #[test]
    fn on_ack_names_the_byte_and_on_read_ack_precedes_the_next_read() {
        let mut dev = Dev::new();
        dev.nack = Some(0x11);
        let mut step = Step::power(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, 0x15 << 1);
        step.ack_clock(&mut dev);
        step.byte(&mut dev, 0x10);
        step.ack_clock(&mut dev);
        step.byte(&mut dev, 0x5A);
        step.ack_clock(&mut dev);
        step.byte(&mut dev, 0x5B);
        step.ack_clock(&mut dev);
        step.stop(&mut dev);
        step.start(&mut dev);
        step.byte(&mut dev, 0x15 << 1);
        step.ack_clock(&mut dev);
        step.byte(&mut dev, 0xA7);
        step.ack_clock(&mut dev);
        step.repeated_start(&mut dev);
        step.byte(&mut dev, (0x15 << 1) | 1);
        step.ack_clock(&mut dev);
        step.read_byte(&mut dev);
        step.master_ack(&mut dev, true);
        step.read_byte(&mut dev);
        step.master_ack(&mut dev, false);
        step.stop(&mut dev);
        assert_eq!(
            dev.events,
            vec![
                Ev::StartWrite,
                Ev::Ack(AckKind::Address, true),
                Ev::Write(0x10),
                Ev::Ack(AckKind::Byte, true),
                Ev::Write(0x5A),
                Ev::Ack(AckKind::Byte, true),
                Ev::Write(0x5B),
                Ev::Ack(AckKind::Byte, false),
                Ev::Stop,
                Ev::StartWrite,
                Ev::Ack(AckKind::Address, true),
                Ev::Write(0xA7),
                Ev::Ack(AckKind::Byte, true),
                Ev::StartRead,
                Ev::Ack(AckKind::Address, true),
                Ev::Read(0xB4),
                Ev::ReadAck(true),
                Ev::Read(0),
                Ev::ReadAck(false),
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
        assert_eq!(
            dev.events,
            vec![
                Ev::StartRead,
                Ev::Ack(AckKind::Address, true),
                Ev::Read(0xB4),
            ]
        );
        step.t += 1_000;
        step.scl = false;
        step.engine
            .edge(Time::from_ps(step.t), false, step.wire(), false, &mut dev);
        step.pump(&mut dev);
        assert!(!step.driven, "the target is not driving after the fall");
        assert_eq!(dev.events.len(), 3, "no read ack and no further byte");
    }
}
