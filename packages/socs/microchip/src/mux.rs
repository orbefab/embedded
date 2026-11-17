//! Pin mux. Every driver of a pin level is a [`DriveSource`].
//!
//! `PORTx` with `DDRx` is the base source. A timer `OCnx` is
//! [`DriveSource::Compare`]. USART `TXD` is [`DriveSource::UsartTx`], MSPIM
//! `XCK` is [`DriveSource::UsartXck`], and SPI is [`DriveSource::Spi`].
//! TWI is not a [`DriveSource`]: while `TWEN` is set, `SDA` and `SCL` follow
//! Table 14-7 through [`PinMux::set_twi_override`], open drain: low while the TWI pulls
//! them, an input otherwise. Override slots are keyed by source, not by timer index.
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

use crate::Port;
use crate::periph::Ctx;
use crate::{Level, PinDrive};

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

    /// No level waits out the synchronizer past `cycle`, so [`Self::visible`] is the same from
    /// `cycle` on.
    fn settled(&self, cycle: u64) -> bool {
        self.pending_at[..usize::from(self.pending_len)]
            .iter()
            .all(|&at| at <= cycle)
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
    /// Clear means `reported` and the pads match the registers and every override.
    /// A change to a pin's inputs reconciles this port or sets this. `plant` sets it.
    /// `set_reset_pin` writes the wire and `reported` together, so the port stays clear.
    /// A store of the same DDR and PORT skips pin work while this is clear.
    stale: bool,
    /// Last level and drive handed to the host. Starts released and off, so reset emits
    /// nothing.
    reported: [(Level, PinDrive); 8],
    wires: [Wire; 8],
    over: [[Option<Drive>; 4]; 8],
    /// Bit set: the USART receiver forces this pin to an input (`RXEN`).
    force_in: u8,
    /// `DIDR0` / `DIDR1`. The digital input buffer is off; `PINx` reads 0.
    buf_off: u8,
    /// `TWEN`. These bits ignore `DDR` and peripheral overrides (Table 14-7).
    twi: u8,
    /// The TWI pulls these `twi` bits low, `SDA_OUT` and `SCL_OUT` clear.
    twi_low: u8,
}

impl PortState {
    const fn new() -> Self {
        Self {
            ddr: 0,
            port: 0,
            stale: false,
            reported: [(Level::Released, PinDrive::Off); 8],
            wires: [Wire::released(); 8],
            over: [[None; 4]; 8],
            force_in: 0,
            buf_off: 0,
            twi: 0,
            twi_low: 0,
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
    /// `RSTDISBL` is unprogrammed, so `PC6` is `RESET`: an input with its own pull-up that
    /// `DDRC6`, `PORTC6`, and `PUD` do not move, and `PINC6` reads 0 (Table 14-7, §14.3.2).
    reset_pin: bool,
}

const RESET_PORT: Port = Port::C;
const RESET_BIT: u8 = 6;

impl PinMux {
    pub(crate) fn new() -> Self {
        let mut mux = Self {
            ports: [PortState::new(); 3],
            pud: false,
            edges: [PadEdge {
                pin: 0,
                level: Level::Released,
                at: 0,
            }; EDGE_CAP],
            edge_len: 0,
            reset_pin: false,
        };
        mux.set_reset_pin(true);
        mux
    }

    /// Whether `PC6` is `RESET`, from the fuses. The pin takes its new state silently, as
    /// at power-on.
    pub(crate) fn set_reset_pin(&mut self, on: bool) {
        self.reset_pin = on;
        let level = self.electrical(RESET_PORT, RESET_BIT);
        let drive = self.drive_of(RESET_PORT, RESET_BIT);
        let digital = self.digital(RESET_PORT, RESET_BIT, level);
        let state = &mut self.ports[RESET_PORT.index()];
        state.wires[usize::from(RESET_BIT)].plant_level(digital);
        state.reported[usize::from(RESET_BIT)] = (level, drive);
    }

    /// The level the host drives on each pin, [`Level::Released`] where it drives none.
    pub(crate) fn host_levels(&self) -> [Level; crate::PIN_COUNT] {
        std::array::from_fn(|id| {
            Port::from_pin(id as u32).map_or(Level::Released, |(port, bit)| {
                self.ports[port.index()].wires[usize::from(bit)].host
            })
        })
    }

    pub(crate) fn is_reset_pin(&self, pin: u32) -> bool {
        self.reset_pin && pin == RESET_PORT.pin_id(RESET_BIT)
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

    /// [`Self::pin_bits`] is the same at `cycle` and every later cycle, until the next pad change.
    pub(crate) fn settled(&self, port: Port, cycle: u64) -> bool {
        self.ports[port.index()]
            .wires
            .iter()
            .all(|wire| wire.settled(cycle))
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

    pub(crate) fn store_gpio(
        &mut self,
        port: Port,
        ddr: u8,
        value: u8,
        ctx: &mut Ctx<'_>,
        at: u64,
    ) {
        if self.store_skips(port, ddr, value) {
            return;
        }
        {
            let state = &mut self.ports[port.index()];
            state.ddr = ddr;
            state.port = value;
        }
        self.reconcile(port, ctx, None, at, true, true);
    }

    /// A store of `ddr` and `value` to `port` changes no pin, so [`Self::store_gpio`] returns
    /// before it does anything.
    pub(crate) fn store_skips(&self, port: Port, ddr: u8, value: u8) -> bool {
        let state = &self.ports[port.index()];
        let skips = !state.stale && state.ddr == ddr && state.port == value;
        #[cfg(feature = "port-skip-check")]
        debug_assert!(
            !skips
                || (0..8u8).all(|bit| {
                    let (digital, reported) = self.target(port, bit);
                    state.wires[usize::from(bit)].pad == digital
                        && state.reported[usize::from(bit)] == reported
                }),
            "skipped store on {port:?} would change a pin"
        );
        skips
    }

    pub(crate) fn poke_gpio(&mut self, port: Port, ddr: u8, value: u8) {
        {
            let state = &mut self.ports[port.index()];
            state.ddr = ddr;
            state.port = value;
        }
        self.plant(port);
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
        self.ports[port.index()].wires[usize::from(bit)].host = level;
        self.reconcile(port, ctx, None, at, false, true);
    }

    /// Table 14-7. While `mask` bits are set, those pads are open-drain TWI
    /// pins: `DDR` and peripheral overrides do not drive them. The `low` bits of `mask` are
    /// pulled low, and the rest are inputs with the `PORTx` pull-up.
    pub(crate) fn set_twi_override(
        &mut self,
        port: Port,
        mask: u8,
        low: u8,
        ctx: &mut Ctx<'_>,
        report_at: Option<u64>,
        at: u64,
    ) {
        let state = &mut self.ports[port.index()];
        let low = low & mask;
        if state.twi == mask && state.twi_low == low {
            return;
        }
        state.twi = mask;
        state.twi_low = low;
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

    /// Current level and drive of every pin. A pull-up is [`Level::High`]. A floating pin
    /// is [`Level::Released`].
    pub(crate) fn levels(&self) -> [Option<(Level, PinDrive)>; crate::PIN_COUNT] {
        let mut out = [None; crate::PIN_COUNT];
        for port in [Port::B, Port::C, Port::D] {
            for bit in 0..8 {
                let id = port.pin_id(bit) as usize;
                if id < out.len() {
                    out[id] = Some((self.electrical(port, bit), self.drive_of(port, bit)));
                }
            }
        }
        out
    }

    /// The pin's own stage, never the host's level.
    fn drive_of(&self, port: Port, bit: u8) -> PinDrive {
        if self.is_reset_pin(port.pin_id(bit)) {
            return PinDrive::PullUp;
        }
        let state = &self.ports[port.index()];
        let mask = 1u8 << bit;
        // `DDOV` = `SDA_OUT`: the low stage, with the `PUOV` pull-up not stamped beside it.
        if state.twi_low & mask != 0 {
            return PinDrive::Low;
        }
        let input = state.twi & mask != 0
            || state.force_in & mask != 0
            || (state.ddr & mask == 0 && !self.usart_driven(port, bit));
        if !input {
            return match self.output_level(port, bit) {
                Level::High => PinDrive::High,
                Level::Low | Level::Released => PinDrive::Low,
            };
        }
        if state.port & mask != 0 && !self.pud {
            PinDrive::PullUp
        } else {
            PinDrive::Off
        }
    }

    /// The pins whose digital input `PINx` and the edge detectors see, one bit per pin id: not
    /// a pin whose buffer `DIDRn` turns off, nor `PC6` while it is `RESET`.
    pub(crate) fn sensing(&self) -> u64 {
        let mut pins = 0u64;
        for port in [Port::B, Port::C, Port::D] {
            let on = !self.ports[port.index()].buf_off;
            pins |= u64::from(on) << port.pin_id(0);
        }
        if self.reset_pin {
            pins &= !(1 << RESET_PORT.pin_id(RESET_BIT));
        }
        pins
    }

    /// What `PINx` and the edge detectors see of `level`.
    fn digital(&self, port: Port, bit: u8, level: Level) -> Level {
        if self.ports[port.index()].buf_off & (1 << bit) != 0 || self.is_reset_pin(port.pin_id(bit))
        {
            Level::Low
        } else {
            level
        }
    }

    /// Poke path: the registers already changed. Settle the pad so a following
    /// peek sees it, and leave `reported` alone so the next real write still
    /// emits.
    fn plant(&mut self, port: Port) {
        for bit in 0..8 {
            let level = if self.is_reset_pin(port.pin_id(bit)) {
                Level::Low
            } else {
                self.electrical(port, bit)
            };
            self.ports[port.index()].wires[usize::from(bit)].plant_level(level);
        }
        self.ports[port.index()].stale = true;
    }

    /// Digital level for the pad, and the level and drive for `reported`.
    fn target(&self, port: Port, bit: u8) -> (Level, (Level, PinDrive)) {
        let level = self.electrical(port, bit);
        (
            self.digital(port, bit, level),
            (level, self.drive_of(port, bit)),
        )
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
            let (digital, now) = self.target(port, bit);
            let pin = port.pin_id(bit);
            let changed = self.ports[port.index()].wires[usize::from(bit)].latch(digital, at);
            if changed && notify {
                self.push_edge(pin as u8, digital, at);
            }
            let reported = &mut self.ports[port.index()].reported[usize::from(bit)];
            if *reported == now {
                continue;
            }
            *reported = now;
            if emit {
                ctx.emit_pin(pin, now, report_at, self.origin_of(port, bit));
            }
        }
        self.ports[port.index()].stale = false;
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
        if self.is_reset_pin(port.pin_id(bit)) {
            return match state.wires[usize::from(bit)].host {
                Level::Released => Level::High,
                host => host,
            };
        }
        // `PUOE` = `TWEN`, `PUOV` = `PORTCn`·¬`PUD`, `PVOE` = `TWEN`, `PVOV` = 0.
        if state.twi_low & mask != 0 {
            return Level::Low;
        }
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
    use crate::bus::Epochs;
    use crate::periph::Ctx;
    use std::collections::VecDeque;

    fn ctx<'a>(
        pins: &'a mut VecDeque<crate::PinReport>,
        pending: &'a mut Vec<crate::PinReport>,
        overflow: &'a mut u64,
        logs: &'a mut Vec<crate::IoWrite>,
        drops: &'a mut u64,
    ) -> Ctx<'a> {
        Ctx::new(pins, pending, overflow, logs, drops, &Epochs::START)
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
