//! Data-space dispatch.
//!
//! Addresses `0x0000`–`0x001F` are the register file. `0x0020`–`0x00FF` is I/O.
//! An address the chip table assigns is `{ device, register id }`, decoded once
//! when the bus is built. Reserved I/O addresses read as zero and ignore writes.
//! SRAM runs from `0x0100` through RAMEND. `SREG`, `SPL`, and `SPH` are the core's.
//!
//! `next_event` is the soonest [`crate::periph::Device::next_event`].
//! An instruction that ends before that event
//! does no device work. An event inside the instruction still runs on that cycle.
//! [`index::Index`] is the device list resolved once at construction, including
//! the flag, comparator, and capture slots.
//! [`hostside::HostSide`]
//! is the host SPI master, not a device on the die.

mod gaps;
pub(crate) mod hostside;
mod index;
mod power;
mod record;
mod seam;

use std::collections::VecDeque;

use hostside::HostSide;
use index::Index;
pub(crate) use power::Epochs;
use seam::Samples;

use crate::Port;
use crate::analog::{HostAnalog, Sense, adc_code};
use orbe_avr::Core;

use crate::chip::{ClockDomain, DEVICES, DeviceSpec, ModuleGate, RAMEND, SPH, SPL, SREG};
use crate::mux::PinMux;
use crate::periph::usart::{Sent, Shifted};
use crate::periph::{Device, Notice, StoreAt, Touch, push_pin, rows_of};
use crate::regs::{GPIO_PIN, OwnedReg};
use crate::{Level, PIN_QUEUE_CAP, PinLevels, PinOrigin, PinReport};

/// Data-space address of I/O register 0.
pub(crate) use orbe_avr::IO_BASE;

const IO_END: u16 = 0x00FF;
const SRAM_BASE: u16 = 0x0100;
const IO_LEN: usize = 0xE0;
// `PinMux::unread` holds one bit per I/O index in four words.
const _: () = assert!(IO_LEN <= 256);
const NO_DEV: u8 = 0xFF;

#[derive(Debug, Clone, Copy)]
struct Slot {
    dev: u8,
    reg: u8,
}

/// Pin reports queued for the host.
#[derive(Debug, Clone)]
struct HostQueues {
    pins: VecDeque<PinReport>,
    pending: Vec<PinReport>,
    pin_overflow: u64,
}

impl HostQueues {
    fn ctx<'a>(&'a mut self, epochs: &'a Epochs) -> crate::periph::Ctx<'a> {
        crate::periph::Ctx::new(
            &mut self.pins,
            &mut self.pending,
            &mut self.pin_overflow,
            epochs,
        )
    }
}

#[derive(Debug)]
pub(crate) struct Bus {
    /// Soonest event. Read on every instruction, so it stays at the front
    /// with the other bytes `tick` touches. `u64::MAX` when nothing is scheduled.
    next_event: u64,
    /// Cycles completed. Edges are stamped against this.
    cycle: u64,
    /// OR of device interrupt requests. Test sources are separate.
    device_irq: u64,
    /// A `TCNT` write is waiting for its instruction's clocks to finish.
    hold_any: bool,
    /// A byte queued during this instruction still has `cycle = None`.
    needs_stamp: bool,
    samples: Samples,
    /// An enable window is waiting for the instruction to retire.
    windows: bool,
    /// Stall cycles the current instruction still has to add (§8.4.1).
    cpu_halt: u8,
    sram: Vec<u8>,
    strobe: [u8; IO_LEN],
    owners: [Slot; IO_LEN],
    devices: Vec<Device>,
    /// Kind of each `devices[i]`, resolved when the chip is built.
    index: Index,
    /// Level of each Table 24-6 trigger flag. Bit 0 is unused.
    trigger_level: u8,
    /// `PRR` bit and sleep clock of `devices[i]`, copied from the die row.
    row_gate: Vec<ModuleGate>,
    mux: PinMux,
    analog: HostAnalog,
    host: HostQueues,
    /// `SLEEP` ran with `SE` set. The facade calls [`Self::begin_sleep`].
    /// Not read on the fetch path: `SLEEP` sets `Core::sleeping` first.
    sleep_armed: bool,
    /// Watchdog timeout asked for a system reset. Set only when an event
    /// fires, and returned from that slow path. Not read on the fetch path.
    wdt_reset: bool,
    /// Host SPI master and the host slave's MISO queue.
    host_side: HostSide,
    /// Oscillator hertz. `cpu_hz` is this divided by `div`.
    osc_hz: u32,
    /// What the firmware reached that the model does not run.
    gaps: crate::gaps::Gaps,
    /// The last two `CLKPR` divisions. CPU cycles stay 1; wall time scales.
    epochs: Epochs,
    /// Schedule-and-ack source for the `irq` tests. Not a device, and not on `tick`.
    #[cfg(test)]
    test_irq: crate::periph::probe::Probe,
}

clone_fields!(Bus {
    next_event,
    cycle,
    device_irq,
    hold_any,
    needs_stamp,
    samples,
    windows,
    cpu_halt,
    sram,
    strobe,
    owners,
    devices,
    index,
    trigger_level,
    row_gate,
    mux,
    analog,
    host,
    sleep_armed,
    wdt_reset,
    host_side,
    osc_hz,
    gaps,
    epochs,
    #[cfg(test)]
    test_irq
});

impl orbe_avr::Bus for Bus {
    #[inline(always)]
    fn load(&mut self, core: &Core, addr: u16) -> u8 {
        if (SRAM_BASE..=RAMEND).contains(&addr) {
            return self.sram[usize::from(addr - SRAM_BASE)];
        }
        if addr <= 0x001F {
            return core.reg(addr as u8);
        }
        if addr == SREG {
            return core.sreg();
        }
        if let Some(byte) = self.sp_byte(core, addr) {
            return byte;
        }
        if let Some(index) = io_index(addr) {
            self.note_read(index);
            if self.gap_watched_load(addr) {
                self.gap_load(core);
            }
            return self.load_io(index);
        }
        0
    }

    #[inline(always)]
    fn store(&mut self, core: &mut Core, addr: u16, value: u8, cycles: u64) {
        self.store(core, addr, value, cycles);
    }

    /// The core peeks only for an instruction it runs, so a peek of `PINx` is the firmware's
    /// read, as a load is.
    #[inline(always)]
    fn peek(&self, core: &Core, addr: u16) -> u8 {
        if let Some(index) = io_index(addr) {
            self.note_read(index);
        }
        self.peek(core, addr)
    }

    /// A [`Self::load`] of `addr` changes nothing and reads what [`Self::peek`] reads. What it
    /// reads changes only at the next event, on a store, or on a host call.
    #[inline(always)]
    fn steady(&self, addr: u16) -> bool {
        let Some(index) = io_index(addr) else {
            return true;
        };
        if addr == SREG || addr == SPL || addr == SPH {
            return true;
        }
        if self.gap_watched_load(addr) {
            return false;
        }
        let slot = self.owners[index];
        slot.dev == NO_DEV
            || self.devices[usize::from(slot.dev)].steady(slot.reg, &self.mux, self.cycle)
    }

    /// A pure device load at `cycle`. The bus cycle lags during a free run, and an 8-bit counter
    /// read is the byte at the cycle the core asks for.
    #[inline(always)]
    fn projected_at(&self, addr: u16, cycle: u64) -> Option<u8> {
        let index = io_index(addr)?;
        let slot = self.owners[index];
        if slot.dev == NO_DEV {
            return None;
        }
        let clock = self.clock_view();
        self.devices[usize::from(slot.dev)].pure(slot.reg, cycle, &clock)
    }

    /// A [`Self::store`] of `value` to `addr` would change no state, pin, log entry, notice, or
    /// schedule.
    #[inline(always)]
    fn store_idle(&self, core: &Core, addr: u16, value: u8) -> bool {
        if (SRAM_BASE..=RAMEND).contains(&addr)
            || addr <= 0x001F
            || addr == SREG
            || addr == SPL
            || addr == SPH
        {
            return self.peek(core, addr) == value;
        }
        let Some(index) = io_index(addr) else {
            return true;
        };
        if self.gap_watched(addr) {
            return false;
        }
        let slot = self.owners[index];
        // A stepped store also delivers waiting edges and notices.
        slot.dev != NO_DEV
            && self.devices[usize::from(slot.dev)].store_idle(slot.reg, value, &self.mux)
            && !self.mux.has_pending_edges()
            && !self.notices_wait()
    }

    /// The stepped store sets `SREG` and then takes no interrupt while none is pending, and a
    /// free run ends before the next event, so none becomes pending inside it.
    #[inline(always)]
    fn store_writes_sreg(&self, addr: u16) -> bool {
        addr == SREG && self.pending_irq() == 0
    }

    /// A store to SRAM reaches no device.
    #[inline(always)]
    fn plain(&self, addr: u16) -> bool {
        (SRAM_BASE..=RAMEND).contains(&addr)
    }

    #[inline(always)]
    fn store_writes_sp(&self, addr: u16) -> bool {
        addr == SPL || addr == SPH
    }

    /// `SBI` writes a one into one bit. `CBI` writes a zero.
    ///
    /// Bits in the strobe mask are not copied from the current value, so a
    /// write-one-to-clear flag register keeps its other flags (DS40002061B §7.4).
    #[inline(always)]
    fn write_bit(&mut self, core: &mut Core, addr: u16, bit: u8, set: bool) {
        let current = self.peek(core, addr);
        let w1c = self.strobe_at(addr);
        let bit_mask = 1u8 << (bit & 7);
        let written = if set {
            (current & !w1c) | bit_mask
        } else {
            (current & !w1c) & !bit_mask
        };
        self.store(core, addr, written, 2);
    }

    #[inline(always)]
    fn sleep(&mut self) -> bool {
        let enabled = match self.system() {
            Some(sys) => sys.se(),
            None => true,
        };
        if enabled {
            self.sleep_armed = true;
        }
        enabled
    }

    #[inline(always)]
    fn watchdog_reset_timer(&mut self) {
        let now = self.cycle;
        if let Some(sys) = self.system_mut() {
            sys.wdr(now);
        }
        self.windows = true;
        self.note_schedule();
    }

    #[inline(always)]
    fn on_spm(&mut self, pc: u16, cycle: u64) {
        // DS40002061B §27.3.1, §27.3.2: `SPM` is disabled in the Application Flash section
        // (`pc < boot_word`) and active only in the Boot Loader Flash section (`BOOTSZ`, Table 27-13).
        if pc >= self.fuses().boot_word {
            self.refuse(crate::Refused::Instruction("spm"), cycle, pc);
        }
    }

    #[inline(always)]
    fn on_break(&mut self, pc: u16, cycle: u64) -> bool {
        // DS40002198B `BREAK`, DS40002061B Table 28-8: with `DWEN` unprogrammed, debugWIRE is
        // disabled and `BREAK` acts as a one-cycle `NOP`.
        if !self.fuses().dwen {
            return false;
        }
        self.refuse(crate::Refused::Instruction("break"), cycle, pc);
        true
    }

    #[inline(always)]
    fn take_stall(&mut self) -> u8 {
        let stall = self.cpu_halt;
        if stall != 0 {
            self.cpu_halt = 0;
        }
        stall
    }
}

impl Bus {
    pub(crate) fn new() -> Self {
        let specs = DEVICES;
        let mut owners = [Slot {
            dev: NO_DEV,
            reg: 0,
        }; IO_LEN];
        let mut strobe = [0u8; IO_LEN];
        let mut devices = Vec::with_capacity(specs.len());
        let mut row_gate = Vec::with_capacity(specs.len());
        let mut timer_index = 0u8;
        let index = Index::from_specs(specs);
        let mut rows: Vec<OwnedReg> = Vec::new();
        for spec in specs {
            let dev_index = u8::try_from(devices.len()).expect("device index fits in a byte");
            let index = timer_index;
            if matches!(spec, DeviceSpec::Timer(_)) {
                timer_index = timer_index.saturating_add(1);
            }
            devices.push(Device::from_spec(spec, index));
            row_gate.push(spec.gate());
            rows_of(spec, &mut rows);
            for row in &rows {
                let slot = usize::from(row.addr - IO_BASE);
                owners[slot] = Slot {
                    dev: dev_index,
                    reg: row.id,
                };
                strobe[slot] = row.strobe;
            }
        }
        let mut pinx = [0u64; 4];
        for (slot, owner) in owners.iter().enumerate() {
            if owner.dev != NO_DEV
                && owner.reg == GPIO_PIN
                && matches!(devices[usize::from(owner.dev)], Device::Port(_))
            {
                pinx[slot >> 6] |= 1 << (slot & 63);
            }
        }
        let sram_len = usize::from(RAMEND - SRAM_BASE) + 1;
        let mut bus = Self {
            next_event: u64::MAX,
            cycle: 0,
            device_irq: 0,
            hold_any: false,
            needs_stamp: false,
            samples: Samples::Store,
            windows: false,
            cpu_halt: 0,
            sram: vec![0; sram_len],
            strobe,
            owners,
            devices,
            index,
            trigger_level: 0,
            row_gate,
            mux: PinMux::new(),
            analog: HostAnalog::new(),
            host: HostQueues {
                pins: VecDeque::with_capacity(PIN_QUEUE_CAP),
                pending: Vec::with_capacity(crate::periph::PENDING_PIN_CAP),
                pin_overflow: 0,
            },
            sleep_armed: false,
            wdt_reset: false,
            host_side: HostSide::idle(),
            osc_hz: 16_000_000,
            gaps: crate::gaps::Gaps::default(),
            epochs: Epochs::START,
            #[cfg(test)]
            test_irq: crate::periph::probe::Probe::new(),
        };
        bus.mux.set_pinx(pinx);
        bus.note_schedule();
        bus
    }

    /// Drive `pin` from the host at the current cycle. [`Level::Released`] is high-Z.
    ///
    /// A later [`Self::set_pin_voltage`] replaces this drive, and a later
    /// `drive_pin` replaces a voltage. The ADC reads a digital high as 5.0 V
    /// and a digital low or a float as 0.
    pub(crate) fn drive_pin(&mut self, pin: u32, level: Level) {
        self.drive_pin_at(pin, level, self.cycle);
    }

    /// [`Self::drive_pin`] from `at`, a cycle at or before this one. The synchronizer and every
    /// edge detector count from `at`.
    pub(crate) fn drive_pin_at(&mut self, pin: u32, level: Level, at: u64) {
        self.analog.set_pin_level(pin, level);
        self.drive_mux(pin, level, at);
        self.refresh_acmp();
        self.note_schedule();
    }

    /// Analog voltage on a GPIO pin, in microvolts. The Schmitt trigger updates
    /// the digital input (Table 30-1). Between `VIL` and `VIH` the previous
    /// level holds.
    pub(crate) fn set_pin_voltage(&mut self, pin: u32, uv: i32) {
        let Some(level) = self.analog.set_pin_voltage(pin, uv) else {
            return;
        };
        self.drive_mux(pin, level, self.cycle);
        self.refresh_acmp();
        self.note_schedule();
    }

    /// Voltage on an ADC channel. A channel with a pad uses that pin, so the
    /// last of `set_pin_voltage` and this call wins.
    pub(crate) fn set_channel_voltage(&mut self, channel: u8, uv: i32) {
        let pin = self.adc().and_then(|adc| adc.channel_pin(channel));
        if let Some(pin) = pin {
            self.set_pin_voltage(u32::from(pin), uv);
            return;
        }
        self.analog.set_only(channel, uv);
        self.refresh_acmp();
        self.note_schedule();
    }

    pub(crate) fn set_avcc(&mut self, uv: i32) {
        self.analog.set_avcc(uv);
    }

    pub(crate) fn set_aref(&mut self, uv: i32) {
        self.analog.set_aref(uv);
    }

    pub(crate) fn set_temperature_milli(&mut self, milli: i32) {
        self.analog.set_temperature_milli(milli);
    }

    /// §24.6. Sleep in ADC noise reduction calls this once the CPU has halted.
    pub(crate) fn adc_noise_reduction(&mut self) {
        let cycle = self.cycle;
        let started = self.adc_mut().is_some_and(|adc| adc.noise_reduction(cycle));
        if started {
            self.note_schedule();
        }
    }

    fn drive_mux(&mut self, pin: u32, level: Level, at: u64) {
        let Bus {
            mux, host, epochs, ..
        } = self;
        let mut ctx = host.ctx(epochs);
        mux.drive(pin, level, &mut ctx, at);
        self.dispatch_edges();
        self.deliver_notices(at);
    }

    /// Stop or start `clkI/O` outside a sleep mode. Sleep uses [`Self::apply_sleep_clocks`].
    pub(crate) fn set_io_clock_running(&mut self, running: bool) {
        let cycle = self.cycle;
        let clock = self.clock_view();
        for dev in &mut self.devices {
            dev.gate(Some(running), None, None, cycle, &clock);
        }
        self.note_schedule();
    }

    /// Advance every device whose event falls in the cycles just spent.
    ///
    /// `true` when that walk fired a watchdog reset. The fast path, an
    /// instruction that ends before `next_event`, does not read `wdt_reset`.
    #[inline(always)]
    pub(crate) fn tick(&mut self, cycles: u64) -> bool {
        if cycles == 0 {
            if self.windows {
                self.finish_windows();
            }
            return false;
        }
        let end = self.cycle.wrapping_add(cycles);
        if !self.hold_any && end < self.next_event {
            self.cycle = end;
            if self.needs_stamp || !self.host.pending.is_empty() {
                self.finish_host(end);
            }
            if self.windows {
                self.finish_windows();
            }
            if !self.hold_any && end < self.next_event {
                return false;
            }
        }
        self.advance_to(end);
        if self.windows {
            self.finish_windows();
        }
        self.wdt_reset
    }

    /// Nothing waits on the next instruction to retire: no `TCNT` hold, no unstamped byte, no
    /// enable window, no unflushed pin report and no halt. [`Self::tick`]'s fast path then only
    /// moves the cycle.
    #[inline(always)]
    pub(crate) fn quiet(&self) -> bool {
        !self.hold_any
            && !self.needs_stamp
            && !self.windows
            && self.cpu_halt == 0
            && self.host.pending.is_empty()
    }

    /// [`Self::tick`]'s fast path for `cycles` spent on a quiet bus that end before the next event.
    #[inline(always)]
    pub(crate) fn pass(&mut self, cycles: u64) {
        self.cycle = self.cycle.wrapping_add(cycles);
    }

    pub(crate) fn cycle(&self) -> u64 {
        self.cycle
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.next_event
    }

    /// Fire every event at or before this cycle, which a pad driven from an earlier one leaves.
    pub(crate) fn fire_overdue(&mut self) {
        if self.next_event <= self.cycle {
            self.advance_to(self.cycle);
        }
    }

    pub(crate) fn pending_irq(&self) -> u64 {
        self.device_irq
    }

    pub(crate) fn ack_irq(&mut self, vector: u8) {
        for dev in &mut self.devices {
            dev.ack(vector);
        }
        #[cfg(test)]
        self.test_irq.ack_vector(vector);
        // A cleared trigger flag is a falling edge. Deliver it here so the
        // level does not stay high until the next timer tick.
        let cycle = self.cycle;
        self.deliver_notices(cycle);
        self.recompute_irq();
    }

    /// Every timer clock calls `step`. Projection tests use this as the reference.
    #[cfg(test)]
    pub(crate) fn arm_stepped_timers(&mut self) {
        let clock = self.clock_view();
        for slot in self.index.timers[..usize::from(self.index.timer_n)]
            .iter()
            .flatten()
        {
            if let Device::Timer(timer) = &mut self.devices[usize::from(*slot)] {
                timer.arm_stepped(&clock);
            }
        }
        self.note_schedule();
    }

    /// `(TCNT, flag byte)` for each timer, in die order.
    #[cfg(test)]
    pub(crate) fn timer_observe(&self) -> Vec<(u16, u8)> {
        let clock = self.clock_view();
        let cycle = self.cycle;
        self.index.timers[..usize::from(self.index.timer_n)]
            .iter()
            .flatten()
            .filter_map(|slot| match &self.devices[usize::from(*slot)] {
                Device::Timer(timer) => Some(timer.observe(cycle, &clock)),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn raise_irq_at(&mut self, vector: u8, at_cycle: u64) {
        let now = self.cycle;
        self.test_irq.raise_at(vector, at_cycle, now);
        self.recompute_irq();
        self.note_schedule();
    }

    #[cfg(test)]
    pub(crate) fn clear_irq(&mut self, vector: u8) {
        self.test_irq.clear(vector);
        self.recompute_irq();
    }

    #[cfg(test)]
    pub(crate) fn irq_acked(&self, vector: u8) -> bool {
        self.test_irq.acked(vector)
    }

    #[cfg(test)]
    pub(crate) fn extint_waiting(&self) -> usize {
        match self.slot_dev(self.index.extint) {
            Some(Device::ExtInt(ext)) => ext.waiting(),
            _ => 0,
        }
    }

    /// Marks each pin of the port whose `PINx` sits at I/O `index` read, as
    /// [`PinMux::note_read`] says, at the first load since a drive last turned off. A steady
    /// load in a free run is marked at the cycle the run began on. Every load and peek passes
    /// here, so any other load costs one bit test.
    #[inline(always)]
    fn note_read(&self, index: usize) {
        if self.mux.unread(index) {
            self.note_port_read(index);
        }
    }

    #[cold]
    #[inline(never)]
    fn note_port_read(&self, index: usize) {
        self.mux.loaded(index);
        let slot = self.owners[index];
        if let Device::Port(port) = &self.devices[usize::from(slot.dev)] {
            let now = self.epochs.oscillator_cycles(self.cycle);
            self.mux.note_read(0xFF << port.port().pin_id(0), now);
        }
    }

    /// Hands the mux the pins an enabled external or pin-change interrupt now watches.
    fn note_sensed(&mut self) {
        let Some(Device::ExtInt(ext)) = self.slot_dev(self.index.extint) else {
            return;
        };
        let sensed = ext.sensed();
        let now = self.epochs.oscillator_cycles(self.cycle);
        self.mux.set_watched(sensed, now);
    }

    /// The oscillator cycle the firmware first read pin `pin` at since its drive last turned
    /// off, as [`PinMux::off_read`] says: a load of its port's `PINx`, or an enabled `INTn` or
    /// pin-change interrupt that watches it, while it is an input with its pull-up off and its
    /// digital input on.
    pub(crate) fn pin_read_at(&self, pin: u32) -> Option<u64> {
        self.mux.off_read(pin)
    }

    /// Each pin's [`Self::pin_read_at`] while its drive is off, for [`Self::carry_off_reads`].
    pub(crate) fn off_reads(&self) -> [u64; crate::PIN_COUNT] {
        self.mux.off_reads()
    }

    /// After a reset, keeps the reads from before it of each pin whose drive stayed off.
    pub(crate) fn carry_off_reads(&mut self, before: [u64; crate::PIN_COUNT]) {
        self.mux.carry_off_reads(before);
    }

    #[cfg(test)]
    pub(crate) fn peripheral_cycles(&self) -> u64 {
        self.cycle
    }

    pub(crate) fn take_pin(&mut self) -> Option<PinReport> {
        self.host.pins.pop_front()
    }

    pub(crate) fn pin_overflow(&self) -> u64 {
        self.host.pin_overflow
    }

    pub(crate) fn pin_levels(&self) -> PinLevels {
        PinLevels::from_array(self.mux.levels())
    }

    /// Ports return to their reset state immediately (§11.1). Report every pin
    /// whose level or drive changed. The new queue is empty; these are its first edges.
    pub(crate) fn note_reset_pins(&mut self, before: PinLevels, at: u64) {
        let after = self.mux.levels();
        for pin in 0..crate::PIN_COUNT as u32 {
            let next = after[pin as usize];
            if before.both(pin) == next {
                continue;
            }
            let Some((level, drive)) = next else {
                continue;
            };
            push_pin(
                &mut self.host.pins,
                &mut self.host.pin_overflow,
                PinReport {
                    pin,
                    level,
                    drive,
                    cycle: at,
                    origin: PinOrigin::Gpio,
                },
            );
        }
    }

    /// Drive `RXD` with `bytes` at the host baud and arm the scheduler.
    pub(crate) fn supply_usart(&mut self, index: u8, bytes: &[u8]) {
        let at = self.cycle;
        let slot = self.index.usart.get(usize::from(index)).copied().flatten();
        {
            let Bus {
                devices,
                mux,
                host,
                epochs,
                ..
            } = self;
            let mut ctx = host.ctx(epochs);
            if let Some(slot) = slot
                && let Some(Device::Usart(usart)) = devices.get_mut(usize::from(slot))
            {
                usart.host_supply(bytes, at, mux, &mut ctx);
            }
        }
        self.dispatch_edges();
        self.note_schedule();
    }

    /// The next MOSI byte from a master chip, at the cycle `SPIF` was set.
    pub(crate) fn take_spi(&mut self) -> Option<(u8, u64)> {
        self.spi_mut()?.take_mosi()
    }

    /// The next TWI phase, stamped when `TWINT` sets or `TWSTO` clears.
    pub(crate) fn take_i2c(&mut self, through: u64) -> Option<crate::periph::twi::I2cStamp> {
        self.twi_mut()?.take(through)
    }

    /// The TWI's finished phases, which an in-run reset keeps.
    pub(crate) fn take_i2c_across_reset(&mut self) -> VecDeque<crate::periph::twi::I2cStamp> {
        self.twi_mut().map(|twi| twi.take_all()).unwrap_or_default()
    }

    pub(crate) fn restore_i2c_across_reset(
        &mut self,
        kept: VecDeque<crate::periph::twi::I2cStamp>,
    ) {
        if let Some(twi) = self.twi_mut() {
            twi.restore(kept);
        }
    }

    fn twi_mut(&mut self) -> Option<&mut crate::periph::twi::Twi> {
        let slot = self.index.twi?;
        match self.devices.get_mut(usize::from(slot)) {
            Some(Device::Twi(twi)) => Some(twi),
            _ => None,
        }
    }

    /// The next MISO byte a host master sampled, at that frame's last SCK edge.
    pub(crate) fn take_spi_host_master(&mut self) -> Option<(u8, u64)> {
        self.host_side.master.take_one()
    }

    fn spi_mut(&mut self) -> Option<&mut crate::periph::Spi> {
        let slot = self.index.spi?;
        match &mut self.devices[usize::from(slot)] {
            Device::Spi(spi) => Some(spi),
            _ => None,
        }
    }

    /// Arm a host master. The first pad event is `at`. `SS` rises one cycle
    /// after the last SCK edge. `false` when `bytes` is empty or a transfer is
    /// already active, and nothing changes. This does
    /// not advance the CPU.
    pub(crate) fn arm_host_spi(
        &mut self,
        at: u64,
        bytes: &[u8],
        period: u32,
        mode: u8,
        lsb: bool,
    ) -> bool {
        if bytes.is_empty() || self.host_side.master.is_active() {
            return false;
        }
        let Some(pins) = self.spi_pins() else {
            return false;
        };
        self.host_side
            .master
            .arm(pins, bytes, u64::from(period), mode, lsb, at);
        self.note_schedule();
        true
    }

    pub(crate) fn set_host_serial(
        &mut self,
        index: u8,
        bit_cycles: u64,
        data_bits: u8,
        parity: u8,
        stop_bits: u8,
    ) {
        if let Some(usart) = self.usart_mut(index) {
            usart.set_host_timing(bit_cycles, data_bits, parity, stop_bits);
        }
    }

    fn spi_pins(&self) -> Option<(u8, u8, u8, u8)> {
        let slot = self.index.spi?;
        match &self.devices[usize::from(slot)] {
            Device::Spi(spi) => Some(spi.pins()),
            _ => None,
        }
    }

    /// The next asynchronous frame whose last bit ended at or before oscillator cycle `through`,
    /// from the first USART that has one.
    pub(crate) fn take_sent(&mut self, through: u64) -> Option<Sent> {
        (0..self.index.usart.len() as u8)
            .find_map(|usart| self.usart_mut(usart)?.take_sent(usart, through))
    }

    /// Each USART's finished bytes, which an in-run reset keeps.
    pub(crate) fn take_sent_across_reset(&mut self) -> Vec<Vec<Shifted>> {
        (0..self.index.usart.len() as u8)
            .map(|usart| {
                self.usart_mut(usart)
                    .map(|usart| usart.take_all_sent())
                    .unwrap_or_default()
            })
            .collect()
    }

    pub(crate) fn restore_sent_across_reset(&mut self, kept: Vec<Vec<Shifted>>) {
        for (usart, sent) in (0..).zip(kept) {
            if let Some(usart) = self.usart_mut(usart) {
                usart.restore_sent(sent);
            }
        }
    }

    pub(crate) fn usart_mut(&mut self, index: u8) -> Option<&mut crate::periph::Usart> {
        let slot = self
            .index
            .usart
            .get(usize::from(index))
            .copied()
            .flatten()?;
        match &mut self.devices[usize::from(slot)] {
            Device::Usart(usart) => Some(usart),
            _ => None,
        }
    }

    #[inline(always)]
    pub(crate) fn peek(&self, core: &Core, addr: u16) -> u8 {
        if (SRAM_BASE..=RAMEND).contains(&addr) {
            return self.sram[usize::from(addr - SRAM_BASE)];
        }
        if addr <= 0x001F {
            return core.reg(addr as u8);
        }
        if addr == SREG {
            return core.sreg();
        }
        if let Some(byte) = self.sp_byte(core, addr) {
            return byte;
        }
        if let Some(index) = io_index(addr) {
            return self.peek_io(index);
        }
        0
    }

    fn notices_wait(&self) -> bool {
        let n = usize::from(self.index.notice_n);
        self.index.notices[..n]
            .iter()
            .copied()
            .flatten()
            .any(|slot| self.devices[usize::from(slot)].notice_waits())
    }

    /// `cycles` is the storing instruction's length. A device write sees the
    /// edge that ends it, and the timers' `TCCRnB` and `GTCCR` take effect
    /// there (Figure 14-4).
    #[inline(always)]
    pub(crate) fn store(&mut self, core: &mut Core, addr: u16, value: u8, cycles: u64) {
        if (SRAM_BASE..=RAMEND).contains(&addr) {
            self.sram[usize::from(addr - SRAM_BASE)] = value;
            return;
        }
        if addr <= 0x001F {
            core.set_reg(addr as u8, value);
            return;
        }
        if addr == SREG {
            core.set_sreg(value);
            return;
        }
        if self.write_sp(core, addr, value) {
            return;
        }
        if let Some(index) = io_index(addr) {
            self.store_io(index, value, cycles);
            if self.gap_watched(addr) {
                self.gap_store(core, addr, value);
            }
        }
    }

    #[inline]
    pub(crate) fn poke(&mut self, core: &mut Core, addr: u16, value: u8) {
        if (SRAM_BASE..=RAMEND).contains(&addr) {
            self.sram[usize::from(addr - SRAM_BASE)] = value;
            return;
        }
        if addr <= 0x001F {
            core.set_reg(addr as u8, value);
            return;
        }
        if addr == SREG {
            core.set_sreg(value);
            return;
        }
        if self.write_sp(core, addr, value) {
            return;
        }
        if let Some(index) = io_index(addr) {
            self.poke_io(index, value);
        }
    }

    #[inline(never)]
    fn advance_to(&mut self, end: u64) {
        loop {
            let at = self.next_event;
            if at > end || at == u64::MAX {
                break;
            }
            self.fire_devices(at);
            self.note_schedule();
        }
        let clock = self.clock_view();
        for dev in &mut self.devices {
            dev.release_hold(end, &clock);
        }
        self.hold_any = false;
        self.cycle = end;
        self.finish_host(end);
        self.recompute_irq();
        self.note_schedule();
    }

    fn fire_devices(&mut self, at: u64) {
        // One pass. A notice that arms another device at this same cycle
        // (comparator → Timer1 capture, then `ICF1` → ADC) is fired by
        // `advance_to` after `note_schedule` sees the new event.
        self.latch_samples(at);
        if self.host_side.master.due(at) {
            self.fire_host_spi(at);
        }
        let clock = self.clock_view();
        {
            let Bus {
                devices,
                mux,
                host,
                epochs,
                ..
            } = self;
            let mut ctx = host.ctx(epochs);
            for dev in devices.iter_mut() {
                if dev.next_event() == at {
                    dev.fire(at, &clock, mux, &mut ctx);
                }
            }
        }
        if let Some(slot) = self.index.prescaler
            && let Device::Prescaler(pre) = &mut self.devices[usize::from(slot)]
            && let Some(value) = pre.take_due(at)
        {
            self.apply_prescaler(value, at);
        }
        #[cfg(test)]
        self.test_irq.fire_at(at);
        // A notice must not skip the pad walk. OC compare can raise a Table
        // 24-6 flag and drive the pin on the same fire (§13 alternate
        // function, §14.3.3 pin change).
        self.dispatch_edges();
        self.deliver_notices(at);
        if self.poll_wdt_reset() {
            self.wdt_reset = true;
        }
    }

    fn fire_host_spi(&mut self, at: u64) {
        let miso = self.host_side.master.miso_pin();
        let miso_high =
            Port::from_pin(u32::from(miso)).is_some_and(|(port, bit)| self.mux.pad_high(port, bit));
        let drives = self.host_side.master.advance(at, miso_high);
        let Bus {
            mux, host, epochs, ..
        } = self;
        let mut ctx = host.ctx(epochs);
        for (pin, level) in drives.iter() {
            mux.drive(u32::from(pin), level, &mut ctx, at);
        }
    }

    /// A CPU store to `GTCCR` resets the divider on the edge that ends its
    /// instruction, the edge a `TCCRnB` store selects its clock on (model
    /// parameter 1). The store lands on that edge (Figure 14-4) and the divider
    /// counts `clkI/O` like the timer it feeds (§17.2), so a timer clock on that
    /// edge still has the old phase. A `PSRSYNC` store that ends at edge E
    /// gives a /N timer its next count at E + N (§17.2, §17.4.1). A poke resets
    /// at once.
    fn apply_prescaler(&mut self, value: u8, now: u64) {
        let old = self.clock_view();
        for dev in &mut self.devices {
            dev.materialize(now, &old);
        }
        if let Some(slot) = self.index.prescaler
            && let Device::Prescaler(pre) = &mut self.devices[usize::from(slot)]
        {
            pre.write(value, now);
        }
        let new_clock = self.clock_view();
        for dev in &mut self.devices {
            dev.reschedule(&new_clock);
        }
        self.note_schedule();
    }

    /// Dispatch a queued pad edge. Notices are delivered either
    /// way. The schedule updates when the store, the queue, or a notice asked
    /// for it. A new `CLKPR` division counts from CPU cycle `div_from`.
    fn finish_store(&mut self, touch: Touch, div_from: u64) {
        self.finish_touch(touch);
        let pending = self.mux.has_pending_edges();
        if pending {
            self.dispatch_edges();
        }
        let delivered = self.deliver_notices(self.cycle);
        if touch.div {
            self.absorb_div(div_from);
        }
        if touch.prr {
            self.absorb_prr();
        }
        if touch.schedule || pending || delivered || touch.div || touch.prr {
            self.note_schedule();
        }
    }

    fn finish_touch(&mut self, touch: Touch) {
        if touch.hold {
            self.hold_any = true;
        }
        if touch.stamp {
            self.needs_stamp = true;
        }
        if touch.halt > 0 {
            self.cpu_halt = self.cpu_halt.saturating_add(touch.halt);
        }
        if touch.windows {
            self.windows = true;
        }
        if touch.irq {
            self.recompute_irq();
        }
    }

    fn finish_host(&mut self, cycle: u64) {
        if self.needs_stamp {
            self.stamp_all(cycle);
            self.needs_stamp = false;
            self.dispatch_edges();
            self.deliver_notices(cycle);
            self.recompute_irq();
            self.note_schedule();
        }
        self.flush(cycle);
    }

    fn note_schedule(&mut self) {
        let mut next = u64::MAX;
        for dev in &self.devices {
            next = next.min(dev.next_event());
        }
        next = next.min(self.host_side.master.next());
        #[cfg(test)]
        {
            next = next.min(self.test_irq.next_event());
        }
        self.next_event = next;
    }

    fn recompute_irq(&mut self) {
        let mut bits = 0u64;
        for dev in &self.devices {
            bits |= dev.irq_bits();
        }
        #[cfg(test)]
        {
            bits |= self.test_irq.pending_bits();
        }
        self.device_irq = bits;
    }

    fn clock_view(&self) -> crate::periph::ClockView {
        let Some(slot) = self.index.prescaler else {
            return crate::periph::ClockView::origin_zero();
        };
        match &self.devices[usize::from(slot)] {
            Device::Prescaler(pre) => pre.view(),
            _ => crate::periph::ClockView::origin_zero(),
        }
    }

    fn stamp_all(&mut self, cycle: u64) {
        let Bus {
            devices,
            mux,
            host,
            epochs,
            ..
        } = self;
        let mut ctx = host.ctx(epochs);
        for dev in devices.iter_mut() {
            dev.stamp(cycle, mux, &mut ctx);
        }
    }

    fn flush(&mut self, cycle: u64) {
        if self.host.pending.is_empty() {
            return;
        }
        let HostQueues {
            pins,
            pending,
            pin_overflow,
            ..
        } = &mut self.host;
        for mut report in pending.drain(..) {
            report.cycle = cycle;
            push_pin(pins, pin_overflow, report);
        }
    }

    fn strobe_at(&self, addr: u16) -> u8 {
        io_index(addr).map(|index| self.strobe[index]).unwrap_or(0)
    }

    fn sp_byte(&self, core: &Core, addr: u16) -> Option<u8> {
        if addr == SPL {
            Some(core.sp() as u8)
        } else if addr == SPH {
            Some((core.sp() >> 8) as u8)
        } else {
            None
        }
    }

    fn write_sp(&self, core: &mut Core, addr: u16, value: u8) -> bool {
        if addr == SPL {
            core.set_sp_lo(value);
            true
        } else if addr == SPH {
            core.set_sp_hi(value);
            true
        } else {
            false
        }
    }

    fn peek_io(&self, index: usize) -> u8 {
        let slot = self.owners[index];
        if slot.dev == NO_DEV {
            return 0;
        }
        let clock = self.clock_view();
        self.devices[usize::from(slot.dev)].peek(slot.reg, &self.mux, self.cycle, &clock)
    }

    fn load_io(&mut self, index: usize) -> u8 {
        let slot = self.owners[index];
        if slot.dev == NO_DEV {
            return 0;
        }
        let cycle = self.cycle;
        let clock = self.clock_view();
        let (value, irq) =
            self.devices[usize::from(slot.dev)].read(slot.reg, &self.mux, cycle, &clock);
        if irq {
            self.recompute_irq();
        }
        value
    }

    fn store_io(&mut self, index: usize, value: u8, cycles: u64) {
        let slot = self.owners[index];
        if slot.dev == NO_DEV {
            return;
        }
        let at = StoreAt {
            cycle: self.cycle,
            end: self.cycle + cycles,
        };
        let clock = self.clock_view();
        let touch = {
            let Bus {
                devices,
                mux,
                host,
                epochs,
                ..
            } = self;
            let mut ctx = host.ctx(epochs);
            devices[usize::from(slot.dev)].write(slot.reg, value, &mut ctx, mux, at, &clock)
        };
        if Some(slot.dev) == self.index.extint {
            self.note_sensed();
        }
        // §9.11 puts the new division in force `T1 + T2` to `T1 + 2·T2` after the write.
        // One more cycle at the old division after the store retires, then the new one, lands
        // the second edge on `T1 + T2` whichever way the clock changes.
        self.finish_store(touch, at.end + 1);
    }

    fn poke_io(&mut self, index: usize, value: u8) {
        let slot = self.owners[index];
        if slot.dev == NO_DEV {
            return;
        }
        if self.index.prescaler == Some(slot.dev) {
            self.apply_prescaler(value, self.cycle);
            return;
        }
        let cycle = self.cycle;
        let clock = self.clock_view();
        let touch = {
            let Bus {
                devices,
                mux,
                host,
                epochs,
                ..
            } = self;
            let mut ctx = host.ctx(epochs);
            devices[usize::from(slot.dev)].poke(slot.reg, value, mux, cycle, &clock, &mut ctx)
        };
        self.finish_store(touch, cycle);
    }

    fn latch_samples(&mut self, at: u64) {
        let Some(index) = self.index.adc else {
            return;
        };
        let request = match self.devices.get(usize::from(index)) {
            Some(Device::Adc(adc)) if adc.sample_due(at) => Some(adc.latched_sense()),
            _ => None,
        };
        let Some((sense, refs)) = request else {
            return;
        };
        if let Samples::Asked { due } = &mut self.samples {
            *due = Some(at);
            return;
        }
        let vin = self.resolve(sense);
        let vref = self.analog.vref(refs);
        let code = adc_code(vin, vref);
        if let Some(adc) = self.adc_mut() {
            adc.hold(code);
        }
    }

    fn resolve(&self, sense: Sense) -> i32 {
        match sense {
            Sense::Mux(channel) => {
                let pin = self.adc().and_then(|adc| adc.channel_pin(channel));
                if let Some(pin) = pin {
                    self.analog.resolve(Sense::Pin(pin))
                } else {
                    self.analog.resolve(Sense::Only(channel))
                }
            }
            other => self.analog.resolve(other),
        }
    }

    /// Forward flag edges and comparator output through the index slots.
    /// `true` when a notice was delivered.
    fn deliver_notices(&mut self, at: u64) -> bool {
        let mut note = Notice::default();
        let n = usize::from(self.index.notice_n);
        let sources = self.index.notices;
        for slot in sources[..n].iter().copied().flatten() {
            note.absorb(self.devices[usize::from(slot)].take_notice());
        }
        if note.is_empty() {
            return false;
        }
        self.trigger_level = (self.trigger_level | note.rose) & !note.fell;
        let level = self.trigger_level;
        let aco_now = self.aco_high();
        if note.acic
            && let Some(slot) = self.index.acic
            && let Device::Timer(timer) = &mut self.devices[usize::from(slot)]
        {
            // Enabling copies the level and does not detect an edge.
            timer.set_acic(note.acic_on, aco_now);
        }
        if note.aco
            && let Some(slot) = self.index.acic
            && let Device::Timer(timer) = &mut self.devices[usize::from(slot)]
            && timer.on_aco(note.aco_high, at)
        {
            let clock = self.clock_view();
            for dev in &mut self.devices {
                dev.reschedule(&clock);
            }
        }
        if note.mux
            && let Some(acmp) = self.acmp_mut()
        {
            acmp.set_mux(note.acme, note.aden, note.channel);
        }
        if (note.rose != 0 || note.fell != 0 || note.retarget)
            && let Some(slot) = self.index.adc
            && let Device::Adc(adc) = &mut self.devices[usize::from(slot)]
        {
            if note.rose != 0 || note.fell != 0 {
                adc.on_edges(note.rose, at);
            }
            if note.retarget {
                adc.retarget(level, at, note.synthesize, note.old_adts);
            }
        }
        if note.mux || note.refresh {
            self.refresh_acmp();
        }
        true
    }

    fn aco_high(&self) -> bool {
        match self.slot_dev(self.index.acmp) {
            Some(Device::Acmp(acmp)) => acmp.output_high(),
            _ => false,
        }
    }

    fn acmp_mut(&mut self) -> Option<&mut crate::periph::Acmp> {
        match self.slot_dev_mut(self.index.acmp)? {
            Device::Acmp(acmp) => Some(acmp),
            _ => None,
        }
    }

    fn refresh_acmp(&mut self) {
        let cycle = self.cycle;
        let query = match self.slot_dev(self.index.acmp) {
            Some(Device::Acmp(acmp)) => Some(acmp.compare()),
            _ => None,
        };
        let Some(query) = query else {
            return;
        };
        let high = if query.disabled {
            false
        } else {
            self.resolve(query.pos) > self.resolve(query.neg)
        };
        let schedule = self
            .acmp_mut()
            .is_some_and(|acmp| acmp.consider(high, cycle));
        if schedule {
            self.note_schedule();
        }
    }

    /// Deliver pad edges, in device order.
    ///
    /// A device may drive a pin in `after_pad`. Those edges are delivered
    /// in the same cycle, up to four passes. Idle SPI does not enter this path.
    fn dispatch_edges(&mut self) {
        let mut schedule = false;
        let mut irq = false;
        for _ in 0..4 {
            let edges = self.mux.take_edges();
            if edges.is_empty() {
                break;
            }
            let mut at = self.cycle;
            for edge in edges {
                at = edge.at;
                for dev in self.devices.iter_mut() {
                    let touch = dev.on_pad(edge.pin, edge.level, edge.at);
                    schedule |= touch.schedule;
                    irq |= touch.irq;
                }
            }
            let Bus {
                devices,
                mux,
                host,
                epochs,
                ..
            } = self;
            let mut ctx = host.ctx(epochs);
            for dev in devices.iter_mut() {
                let touch = dev.after_pad(mux, &mut ctx, at);
                schedule |= touch.schedule;
                irq |= touch.irq;
            }
        }
        if schedule {
            let clock = self.clock_view();
            for dev in &mut self.devices {
                dev.reschedule(&clock);
            }
            self.note_schedule();
        }
        if irq {
            self.recompute_irq();
        }
    }
}

/// `None` for `clkCPU`: sleep halts the core and the row has no other gate.
/// `clkASY` follows `clkI/O` because Timer2's async oscillator is unmodeled.
fn domain_running(domain: ClockDomain, io: bool, adc: bool) -> Option<bool> {
    match domain {
        ClockDomain::Io | ClockDomain::Asy => Some(io),
        ClockDomain::Adc => Some(adc),
        ClockDomain::Cpu => None,
    }
}

fn io_index(addr: u16) -> Option<usize> {
    if (IO_BASE..=IO_END).contains(&addr) {
        Some(usize::from(addr - IO_BASE))
    } else {
        None
    }
}
