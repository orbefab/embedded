//! `GTCCR` and the two free-running prescalers (§17, §18).
//!
//! The synchronous divider clocks Timer0 and Timer1. The asynchronous divider
//! clocks Timer2. Both count `clkI/O` unless `TSM` is holding a reset. A timer
//! asks for the divider's origin and derives its own count; this device has no
//! per-cycle work.

use crate::chip::TimerClock;
use crate::regs::GTCCR;

/// `GTCCR.TSM`. Holds a prescaler-reset bit until it is written zero.
const TSM: u8 = 1 << 7;
/// `GTCCR.PSRASY`. Resets Timer2's prescaler.
const PSRASY: u8 = 1 << 1;
/// `GTCCR.PSRSYNC`. Resets the synchronous prescaler.
const PSRSYNC: u8 = 1 << 0;

/// Divider position at a CPU cycle. Copied out of the device so a timer can
/// schedule without borrowing the prescaler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClockView {
    sync_origin: u64,
    async_origin: u64,
    sync_held: bool,
    async_held: bool,
}

impl ClockView {
    pub(crate) const fn origin_zero() -> Self {
        Self {
            sync_origin: 0,
            async_origin: 0,
            sync_held: false,
            async_held: false,
        }
    }

    pub(crate) fn held(self, clock: TimerClock) -> bool {
        match clock {
            TimerClock::Sync => self.sync_held,
            TimerClock::Async => self.async_held,
        }
    }

    pub(crate) fn origin(self, clock: TimerClock) -> u64 {
        match clock {
            TimerClock::Sync => self.sync_origin,
            TimerClock::Async => self.async_origin,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Prescaler {
    sync_origin: u64,
    async_origin: u64,
    sync_held: bool,
    async_held: bool,
    tsm: bool,
    io_on: bool,
    frozen_at: u64,
    /// A CPU store and the edge that ends its instruction.
    pending: Option<(u8, u64)>,
}

impl Prescaler {
    pub(crate) fn new() -> Self {
        Self {
            sync_origin: 0,
            async_origin: 0,
            sync_held: false,
            async_held: false,
            tsm: false,
            io_on: true,
            frozen_at: 0,
            pending: None,
        }
    }

    /// §11.1. Both dividers read phase 0 at the cycle internal reset releases.
    pub(crate) fn release_epoch(&mut self, now: u64) {
        self.sync_origin = now;
        self.async_origin = now;
        self.sync_held = false;
        self.async_held = false;
        self.io_on = true;
        self.frozen_at = now;
    }

    /// Sleep stops `clkI/O`. The divider phase resumes where it stopped.
    pub(crate) fn set_io_clock(&mut self, running: bool, now: u64) {
        if running == self.io_on {
            return;
        }
        if !running {
            self.frozen_at = now;
            self.io_on = false;
            return;
        }
        let delta = now.wrapping_sub(self.frozen_at);
        self.sync_origin = self.sync_origin.wrapping_add(delta);
        self.async_origin = self.async_origin.wrapping_add(delta);
        self.io_on = true;
    }

    pub(crate) fn view(&self) -> ClockView {
        ClockView {
            sync_origin: self.sync_origin,
            async_origin: self.async_origin,
            sync_held: self.sync_held,
            async_held: self.async_held,
        }
    }

    pub(crate) fn peek(&self) -> u8 {
        let mut value = 0;
        if self.tsm {
            value |= TSM;
        }
        if self.async_held {
            value |= PSRASY;
        }
        if self.sync_held {
            value |= PSRSYNC;
        }
        value
    }

    /// A store lands at its retiring edge, which is an event.
    pub(crate) fn steady(&self, reg: u8) -> bool {
        reg == GTCCR
    }

    pub(crate) fn defer(&mut self, value: u8, at: u64) {
        self.pending = Some((value, at));
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.pending.map_or(u64::MAX, |(_, at)| at)
    }

    pub(crate) fn take_due(&mut self, at: u64) -> Option<u8> {
        let (value, due) = self.pending?;
        if due != at {
            return None;
        }
        self.pending = None;
        Some(value)
    }

    /// §17.4.1. `TSM` keeps a written reset asserted until it is cleared.
    pub(crate) fn write(&mut self, value: u8, now: u64) {
        let tsm = value & TSM != 0;
        if value & PSRSYNC != 0 {
            self.sync_origin = now;
            self.sync_held = tsm;
        }
        if value & PSRASY != 0 {
            self.async_origin = now;
            self.async_held = tsm;
        }
        if !tsm {
            if self.sync_held {
                self.sync_origin = now;
                self.sync_held = false;
            }
            if self.async_held {
                self.async_origin = now;
                self.async_held = false;
            }
        }
        self.tsm = tsm;
    }
}
