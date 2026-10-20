//! Pin mux. Every driver of a pin level is a [`DriveSource`].
//!
//! `PORTx` with `DDRx` is the base source. A timer `OCnx` is
//! [`DriveSource::Compare`]. USART `TXD` is [`DriveSource::UsartTx`], MSPIM
//! `XCK` is [`DriveSource::UsartXck`], and SPI is [`DriveSource::Spi`].
//! TWI is not a [`DriveSource`]: while `TWEN` is set, `SDA` and `SCL` follow
//! Table 14-7 through [`PinMux::set_twi_override`]. Override slots are keyed
//! by source, not by timer index.
//!
//! An input's electrical level is the host drive, or high when the pull-up is
//! on (`PORTx` = 1, `DDRx` = 0, `MCUCR.PUD` = 0, DS40002061B Table 14-1), or
//! [`Level::Released`] when the pin floats. A floating pin reads as 0. That
//! read is undefined on the die.
//!
//! `PINx` is that level after the synchronizer. Figure 14-3 delays an external
//! change by ½ to 1½ system clocks, and Figure 14-4 delays a software write by
//! one clock, so the instruction after the write still reads the old value.
//! A change applied at cycle `T` is first visible to a read whose instruction
//! starts at `T + 2`. [`PIN_SYNC_DELAY`] is that count.
//!
//! Input edges are queued here. The bus delivers each one to every device.
//! External interrupt, pin change, input capture, and `T0`/`T1` read them.
//! ADC auto-trigger is a flag event on the bus.

use crate::Level;
use crate::Port;
use crate::periph::Ctx;

/// Cycles from a pad change until `PINx` and the on-chip edge sample see it.
///
/// DS40002061B §14.2.4. `T + 2` is the first instruction boundary past both
/// the 1½-clock maximum and the one-clock software delay plus the following
/// instruction.
pub(crate) const PIN_SYNC_DELAY: u64 = 2;

/// One driver of a pin level, other than the port register itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriveSource {
    Compare {
        timer: u8,
        unit: u8,
    },
    /// USART `TXD`. While this source is connected the pin is an output,
    /// whatever `DDR` says (DS40002061B Table 14-11, `DDOE = TXEN`, `DDOV = 1`).
    UsartTx {
        pin: u8,
    },
    /// USART `XCK` in MSPIM. `DDR` still gates the pin (§21.3).
    UsartXck {
        pin: u8,
    },
    /// SPI `MOSI`, `SCK`, or slave `MISO`. `DDR` still gates the pin.
    /// Table 19-1 forces the other direction with [`PinMux::set_force_input`].
    Spi {
        pin: u8,
    },
}

/// A pad change. `at` is the cycle the level took effect, before the synchronizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PadEdge {
    pub pin: u8,
    pub level: Level,
    pub at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Drive {
    source: DriveSource,
    level: Level,
}

/// Levels waiting out Figure 14-3. Delay is 2, and one change per cycle, so
/// two entries cover the window. Four leaves room for a same-cycle replace.
const PENDING_CAP: usize = 4;

#[derive(Debug, Clone, Copy)]
struct Wire {
    pad: Level,
    /// Last level whose synchronizer delay has elapsed.
    shown: Level,
    pending: [Level; PENDING_CAP],
    pending_at: [u64; PENDING_CAP],
    pending_len: u8,
    host: Level,
}

impl Wire {
    const fn released() -> Self {
        Self {
            pad: Level::Released,
            shown: Level::Released,
            pending: [Level::Released; PENDING_CAP],
            pending_at: [0; PENDING_CAP],
            pending_len: 0,
            host: Level::Released,
        }
    }

    /// `PINx` at `cycle`. A level is visible only once its own `T + 2` has
    /// arrived. A later change does not publish the earlier one early, and
    /// does not replace it before that deadline.
    fn visible(&self, cycle: u64) -> Level {
        let mut shown = self.shown;
        for index in 0..usize::from(self.pending_len) {
            if self.pending_at[index] <= cycle {
                shown = self.pending[index];
            } else {
                break;
            }
        }
        shown
    }

    fn settle(&mut self, cycle: u64) {
        let mut keep = 0u8;
        for index in 0..usize::from(self.pending_len) {
            if self.pending_at[index] <= cycle {
                self.shown = self.pending[index];
            } else {
                let dest = usize::from(keep);
                self.pending[dest] = self.pending[index];
                self.pending_at[dest] = self.pending_at[index];
                keep += 1;
            }
        }
        self.pending_len = keep;
    }

    /// Record a new electrical level at `at`. Two changes in the same cycle
    /// share one sample: the last level is the one the synchronizer will show.
    fn latch(&mut self, level: Level, at: u64) -> bool {
        if self.pad == level {
            return false;
        }
        self.pad = level;
        self.settle(at);
        let visible_at = at.saturating_add(PIN_SYNC_DELAY);
        if self.pending_len > 0 {
            let last = usize::from(self.pending_len) - 1;
            if self.pending_at[last] == visible_at {
                self.pending[last] = level;
                return true;
            }
        }
        if usize::from(self.pending_len) >= PENDING_CAP {
            self.shown = self.pending[0];
            for index in 1..PENDING_CAP {
                self.pending[index - 1] = self.pending[index];
                self.pending_at[index - 1] = self.pending_at[index];
            }
            self.pending_len -= 1;
        }
        let index = usize::from(self.pending_len);
        self.pending[index] = level;
        self.pending_at[index] = visible_at;
        self.pending_len += 1;
        true
    }

    fn plant_level(&mut self, level: Level) {
        self.pad = level;
        self.shown = level;
        self.pending_len = 0;
    }
}

#[derive(Debug, Clone, Copy)]
struct PortState {
    ddr: u8,
    port: u8,
    /// Last level handed to the host. Starts released so reset emits nothing.
    reported: [Level; 8],
    wires: [Wire; 8],
    over: [[Option<Drive>; 4]; 8],
    /// Bit set: the USART receiver forces this pin to an input (`RXEN`).
    force_in: u8,
    /// `DIDR0` / `DIDR1`. The digital input buffer is off; `PINx` reads 0.
    buf_off: u8,
    /// `TWEN`. These bits ignore `DDR` and peripheral overrides (Table 14-7).
    twi: u8,
}

impl PortState {
    const fn new() -> Self {
        Self {
            ddr: 0,
            port: 0,
            reported: [Level::Released; 8],
            wires: [Wire::released(); 8],
            over: [[None; 4]; 8],
            force_in: 0,
            buf_off: 0,
            twi: 0,
        }
    }
}

const EDGE_CAP: usize = 40;

#[derive(Debug, Clone)]
pub(crate) struct PinMux {
    ports: [PortState; 3],
    /// `MCUCR.PUD`. Set disables every pull-up.
    pud: bool,
    edges: [PadEdge; EDGE_CAP],
    edge_len: u8,
    /// A host drive on a TWI pin while `TWEN` is set has already been logged.
    twi_host_logged: bool,
}

impl PinMux {
    pub(crate) fn new() -> Self {
        Self {
            ports: [PortState::new(); 3],
            pud: false,
            edges: [PadEdge {
                pin: 0,
                level: Level::Released,
                at: 0,
            }; EDGE_CAP],
            edge_len: 0,
            twi_host_logged: false,
        }
    }

    pub(crate) fn has_pending_edges(&self) -> bool {
        self.edge_len != 0
    }

    pub(crate) fn take_edges(&mut self) -> Vec<PadEdge> {
        let n = usize::from(self.edge_len);
        self.edge_len = 0;
        self.edges[..n].to_vec()
    }

    pub(crate) fn ddr_port(&self, port: Port) -> (u8, u8) {
        let state = &self.ports[port.index()];
        (state.ddr, state.port)
    }

    /// `PINx` after the synchronizer, at `cycle`.
    pub(crate) fn pin_bits(&self, port: Port, cycle: u64) -> u8 {
        let state = &self.ports[port.index()];
        let mut bits = 0u8;
        for bit in 0..8u8 {
            if state.buf_off & (1 << bit) != 0 {
                continue;
            }
            if state.wires[usize::from(bit)].visible(cycle) == Level::High {
                bits |= 1 << bit;
            }
        }
        bits
    }

    pub(crate) fn set_pud(&mut self, pud: bool, ctx: &mut Ctx<'_>, at: u64, emit: bool) {
        if self.pud == pud {
            return;
        }
        self.pud = pud;
        for port in [Port::B, Port::C, Port::D] {
            self.reconcile(port, ctx, None, at, emit, emit);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_gpio(
        &mut self,
        port: Port,
        ddr: u8,
        value: u8,
        ctx: &mut Ctx<'_>,
        report_at: Option<u64>,
        at: u64,
        emit: bool,
    ) {
        {
            let state = &mut self.ports[port.index()];
            state.ddr = ddr;
            state.port = value;
        }
        if emit {
            self.reconcile(port, ctx, report_at, at, true, true);
        } else {
            self.plant(port, at);
        }
    }

    pub(crate) fn toggle_gpio(&mut self, port: Port, mask: u8, ctx: &mut Ctx<'_>, at: u64) {
        let state = &mut self.ports[port.index()];
        state.port ^= mask;
        self.reconcile(port, ctx, None, at, true, true);
    }

    /// `DIDR0` / `DIDR1`. While `off`, `PINx` reads 0 and devices see low
    /// (§24.9.5, §23.3.3). The output driver is unchanged.
    pub(crate) fn set_buffer_disabled(&mut self, pin: u8, off: bool, ctx: &mut Ctx<'_>, at: u64) {
        let Some((port, bit)) = Port::from_pin(u32::from(pin)) else {
            return;
        };
        let mask = 1u8 << bit;
        let was = self.ports[port.index()].buf_off & mask != 0;
        if was == off {
            return;
        }
        if off {
            self.ports[port.index()].buf_off |= mask;
        } else {
            self.ports[port.index()].buf_off &= !mask;
        }
        self.reconcile(port, ctx, None, at, false, true);
    }

    /// Drive one pin from the host. [`Level::Released`] is high-Z.
    ///
    /// A host report is not emitted for the drive itself. Clearing `DDR` later
    /// reports the level the pin settles to. An output the host also drives
    /// keeps the MCU level.
    pub(crate) fn drive(&mut self, pin: u32, level: Level, ctx: &mut Ctx<'_>, at: u64) {
        let Some((port, bit)) = Port::from_pin(pin) else {
            return;
        };
        let twi = self.ports[port.index()].twi & (1u8 << bit) != 0;
        if twi && matches!(level, Level::High | Level::Low) && !self.twi_host_logged {
            self.twi_host_logged = true;
            // Once per run. The TWI state machine does not sample this drive.
            ctx.log(0x00BC, bit);
        }
        self.ports[port.index()].wires[usize::from(bit)].host = level;
        self.reconcile(port, ctx, None, at, false, true);
    }

    /// Table 14-7. While `mask` bits are set, those pads are open-drain TWI
    /// pins: `DDR` and peripheral overrides do not drive them.
    pub(crate) fn set_twi_override(
        &mut self,
        port: Port,
        mask: u8,
        ctx: &mut Ctx<'_>,
        report_at: Option<u64>,
        at: u64,
    ) {
        let state = &mut self.ports[port.index()];
        if state.twi == mask {
            return;
        }
        state.twi = mask;
        self.reconcile(port, ctx, report_at, at, true, true);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_override(
        &mut self,
        source: DriveSource,
        port: Port,
        bit: u8,
        level: Option<Level>,
        ctx: &mut Ctx<'_>,
        report_at: Option<u64>,
        at: u64,
    ) {
        let changed = {
            let Some(row) = self
                .ports
                .get_mut(port.index())
                .and_then(|state| state.over.get_mut(usize::from(bit)))
            else {
                return;
            };
            let mut found = None;
            let mut empty = None;
            for (index, slot) in row.iter().enumerate() {
                match slot {
                    Some(drive) if drive.source == source => {
                        found = Some(index);
                        break;
                    }
                    None if empty.is_none() => empty = Some(index),
                    _ => {}
                }
            }
            if let Some(level) = level {
                let drive = Drive { source, level };
                if let Some(index) = found {
                    if row[index] == Some(drive) {
                        false
                    } else {
                        row[index] = Some(drive);
                        true
                    }
                } else if let Some(index) = empty {
                    row[index] = Some(drive);
                    true
                } else {
                    false
                }
            } else if let Some(index) = found {
                row[index] = None;
                true
            } else {
                false
            }
        };
        if changed {
            self.reconcile(port, ctx, report_at, at, true, true);
        }
    }

    /// `RXEN` forces the pin to an input. The pull-up stays under `PORTx`
    /// (DS40002061B §14.3.3 and Table 14-11, `PUO = PORTD0 • PUD`).
    pub(crate) fn set_force_input(
        &mut self,
        port: Port,
        bit: u8,
        force: bool,
        ctx: &mut Ctx<'_>,
        report_at: Option<u64>,
        at: u64,
    ) {
        let mask = 1u8 << (bit & 7);
        let state = &mut self.ports[port.index()];
        let was = state.force_in & mask != 0;
        if was == force {
            return;
        }
        if force {
            state.force_in |= mask;
        } else {
            state.force_in &= !mask;
        }
        self.reconcile(port, ctx, report_at, at, true, true);
    }

    /// Electrical level now, not the synchronized `PINx` value.
    pub(crate) fn pad_high(&self, port: Port, bit: u8) -> bool {
        self.pad_level(port, bit) == Level::High
    }

    /// Electrical level now, including [`Level::Released`].
    pub(crate) fn pad_level(&self, port: Port, bit: u8) -> Level {
        self.electrical(port, bit & 7)
    }

    /// Current driven level of every pin. `None` is not driven by the MCU.
    /// A pull-up is [`Level::High`]. A floating pin is [`Level::Released`].
    pub(crate) fn levels(&self) -> [Option<Level>; crate::PIN_COUNT] {
        let mut out = [None; crate::PIN_COUNT];
        for port in [Port::B, Port::C, Port::D] {
            for bit in 0..8 {
                let id = port.pin_id(bit) as usize;
                if id < out.len() {
                    let level = self.electrical(port, bit);
                    out[id] = Some(level);
                }
            }
        }
        out
    }

    /// Poke path: the registers already changed. Settle the pad so a following
    /// peek sees it, and leave `reported` alone so the next real write still
    /// emits.
    fn plant(&mut self, port: Port, _at: u64) {
        for bit in 0..8 {
            let level = self.electrical(port, bit);
            self.ports[port.index()].wires[usize::from(bit)].plant_level(level);
        }
    }

    fn reconcile(
        &mut self,
        port: Port,
        ctx: &mut Ctx<'_>,
        report_at: Option<u64>,
        at: u64,
        emit: bool,
        notify: bool,
    ) {
        for bit in 0..8 {
            let level = self.electrical(port, bit);
            let digital = if self.ports[port.index()].buf_off & (1 << bit) != 0 {
                Level::Low
            } else {
                level
            };
            let pin = port.pin_id(bit);
            let changed = self.ports[port.index()].wires[usize::from(bit)].latch(digital, at);
            if changed && notify {
                self.push_edge(pin as u8, digital, at);
            }
            let reported = &mut self.ports[port.index()].reported[usize::from(bit)];
            if *reported == level {
                continue;
            }
            *reported = level;
            if emit {
                ctx.emit_pin(pin, level, report_at, self.origin_of(port, bit));
            }
        }
    }

    fn push_edge(&mut self, pin: u8, level: Level, at: u64) {
        if usize::from(self.edge_len) >= EDGE_CAP {
            return;
        }
        self.edges[usize::from(self.edge_len)] = PadEdge { pin, level, at };
        self.edge_len += 1;
    }

    fn usart_driven(&self, port: Port, bit: u8) -> bool {
        self.ports[port.index()].over[usize::from(bit)]
            .iter()
            .flatten()
            .any(|drive| matches!(drive.source, DriveSource::UsartTx { .. }))
    }

    fn origin_of(&self, port: Port, bit: u8) -> crate::PinOrigin {
        let mut spi = false;
        for drive in self.ports[port.index()].over[usize::from(bit)]
            .iter()
            .flatten()
        {
            match drive.source {
                DriveSource::UsartTx { .. } | DriveSource::UsartXck { .. } => {
                    return crate::PinOrigin::Usart;
                }
                DriveSource::Spi { .. } => spi = true,
                DriveSource::Compare { .. } => {}
            }
        }
        if spi {
            crate::PinOrigin::Spi
        } else {
            crate::PinOrigin::Gpio
        }
    }

    fn electrical(&self, port: Port, bit: u8) -> Level {
        let state = &self.ports[port.index()];
        let mask = 1u8 << bit;
        // `PUOE` = `TWEN`, `PUOV` = `PORTCn`·¬`PUD`, `PVOE` = `TWEN`, `PVOV` = 0.
        if state.twi & mask != 0 {
            return self.input_level(port, bit);
        }
        if state.force_in & mask != 0 {
            return self.input_level(port, bit);
        }
        if state.ddr & mask != 0 || self.usart_driven(port, bit) {
            return self.output_level(port, bit);
        }
        self.input_level(port, bit)
    }

    fn input_level(&self, port: Port, bit: u8) -> Level {
        let state = &self.ports[port.index()];
        let mask = 1u8 << bit;
        match state.wires[usize::from(bit)].host {
            Level::High | Level::Low => state.wires[usize::from(bit)].host,
            Level::Released if state.port & mask != 0 && !self.pud => Level::High,
            Level::Released => Level::Released,
        }
    }

    fn output_level(&self, port: Port, bit: u8) -> Level {
        let state = &self.ports[port.index()];
        let mask = 1u8 << bit;
        let mut first = None;
        let mut second = None;
        let mut ranked: [Option<Drive>; 4] = [None; 4];
        let mut n = 0;
        for drive in state.over[usize::from(bit)].into_iter().flatten() {
            ranked[n] = Some(drive);
            n += 1;
        }
        ranked[..n].sort_by_key(|drive| rank((*drive).unwrap().source));
        for drive in ranked[..n].iter().flatten() {
            if first.is_none() {
                first = Some(drive.level);
            } else {
                second = Some(drive.level);
                break;
            }
        }
        let Some(only) = first else {
            return if state.port & mask == 0 {
                Level::Low
            } else {
                Level::High
            };
        };
        second.unwrap_or(only)
    }
}

fn rank(source: DriveSource) -> u16 {
    match source {
        DriveSource::Compare { timer, unit } => (u16::from(timer) << 8) | u16::from(unit),
        DriveSource::UsartTx { pin } => 0xE000 | u16::from(pin),
        DriveSource::UsartXck { pin } => 0xE100 | u16::from(pin),
        DriveSource::Spi { pin } => 0xF000 | u16::from(pin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::periph::Ctx;
    use std::collections::VecDeque;

    fn ctx<'a>(
        pins: &'a mut VecDeque<crate::PinReport>,
        pending: &'a mut Vec<crate::PinReport>,
        overflow: &'a mut u64,
        logs: &'a mut Vec<crate::IoWrite>,
        drops: &'a mut u64,
    ) -> Ctx<'a> {
        Ctx::new(pins, pending, overflow, logs, drops)
    }

    #[test]
    fn a_host_edge_is_queued() {
        let mut mux = PinMux::new();
        let mut pins = VecDeque::new();
        let mut pending = Vec::new();
        let mut overflow = 0u64;
        let mut logs = Vec::new();
        let mut drops = 0u64;
        let mut ctx = ctx(
            &mut pins,
            &mut pending,
            &mut overflow,
            &mut logs,
            &mut drops,
        );
        mux.drive(5, Level::High, &mut ctx, 3);
        let edges = mux.take_edges();
        assert_eq!(
            edges,
            vec![PadEdge {
                pin: 5,
                level: Level::High,
                at: 3,
            }]
        );
        assert!(pins.is_empty(), "the host drive is not a model report");
        assert_eq!(mux.pin_bits(Port::B, 3) & 0x20, 0, "same cycle reads 0");
        assert_eq!(mux.pin_bits(Port::B, 4) & 0x20, 0, "the next cycle reads 0");
        assert_ne!(
            mux.pin_bits(Port::B, 5) & 0x20,
            0,
            "visible two cycles later"
        );
    }
}
