//! The firmware's accesses the model does not run, checked where the bus sees them. Each check
//! cites DS40002061B for why a run past it would be wrong, or why it stays right.
//!
//! Only the core's loads and stores come here. A host poke is no firmware access.

use orbe_avr::Core;

use super::Bus;
use crate::gaps::{Gaps, Noted, Refusal, Refused, Unmodeled};

const ACSR: u16 = 0x0050;
const MCUCR: u16 = 0x0055;
const SPMCSR: u16 = 0x0057;
const OSCCAL: u16 = 0x0066;
const ADCSRB: u16 = 0x007B;
const ADMUX: u16 = 0x007C;
const ASSR: u16 = 0x00B6;
const TWBR: u16 = 0x00B8;
const TWSR: u16 = 0x00B9;
const TWAR: u16 = 0x00BA;
const TWCR: u16 = 0x00BC;
const UCSR0C: u16 = 0x00C2;

/// §23.3.2 `ACSR`.
const ACD: u8 = 1 << 7;
const ACBG: u8 = 1 << 6;
const ACIE: u8 = 1 << 3;
const ACIC: u8 = 1 << 2;

/// CPU cycles after a load of `ACSR` within which a store that sets `ACD` shows the load was
/// the read of a read-modify-write that turns the comparator off, as `ACSR |= _BV(ACD)` is. The
/// sequence is three to five cycles. The rest leaves room for an interrupt between its load and
/// its store. The bound is the model's, not a datasheet figure.
const ACSR_RMW_CYCLES: u64 = 1024;

/// Whether a store to `addr` is one [`Bus::gap_store`] checks.
#[inline(always)]
fn watched(addr: u16) -> bool {
    addr >= ACSR
        && matches!(
            addr,
            ACSR | MCUCR
                | SPMCSR
                | OSCCAL
                | ADCSRB
                | ADMUX
                | ASSR
                | TWBR
                | TWSR
                | TWAR
                | TWCR
                | UCSR0C
        )
}

impl Bus {
    pub(crate) fn gaps(&self) -> &Gaps {
        &self.gaps
    }

    pub(crate) fn watch_gaps(&mut self) {
        self.gaps.on = true;
    }

    /// A store to `addr` is one the checks see, so it never runs inside a free run.
    #[inline(always)]
    pub(super) fn gap_watched(&self, addr: u16) -> bool {
        watched(addr) && self.gaps.on
    }

    /// A load of `addr` is one [`Self::gap_load`] checks, so it never runs inside a free run:
    /// `ACSR` while the comparator is on and nothing is refused yet.
    #[inline(always)]
    pub(super) fn gap_watched_load(&self, addr: u16) -> bool {
        addr == ACSR && self.gaps.on && self.gaps.refusal.is_none() && self.comparator_on()
    }

    pub(crate) fn refusal(&self) -> Option<Refusal> {
        self.gaps.refusal
    }

    /// The notes and the refusal, which an in-run reset keeps.
    pub(crate) fn take_gaps_across_reset(&mut self) -> Gaps {
        self.gaps.across_reset()
    }

    pub(crate) fn restore_gaps(&mut self, gaps: Gaps) {
        self.gaps = gaps;
    }

    /// Stops the chip at `what`, from the instruction at word `pc` at CPU cycle `cycle`.
    pub(crate) fn refuse(&mut self, what: Refused, cycle: u64, pc: u16) {
        let at = self.oscillator_cycles(cycle);
        self.gaps.refuse(Refusal { what, at, pc });
    }

    fn note(&mut self, what: Unmodeled, pc: u16, write: bool) {
        let at = self.oscillator_cycles(self.cycle);
        self.gaps.note(Noted {
            what,
            at,
            pc,
            write,
        });
    }

    fn io_byte(&self, addr: u16) -> u8 {
        super::io_index(addr).map_or(0, |index| self.peek_io(index))
    }

    /// The comparator is on: `ACD` is clear, as it is from reset (§23.3.2).
    pub(super) fn comparator_on(&self) -> bool {
        self.io_byte(ACSR) & ACD == 0
    }

    /// A load of `ACSR`. The model compares the levels the host drives on `AIN0` and `AIN1`, 0 V
    /// or VCC, not their voltages (§23.1: `ACO` is set when `AIN0` is above `AIN1`), so a read
    /// of `ACO` or `ACI` while the comparator is on can be wrong. The load alone does not say
    /// whether the firmware keeps those bits: `ACSR |= _BV(ACD)` loads `ACSR` to turn the
    /// comparator off. So the load waits: a store that sets `ACD` within [`ACSR_RMW_CYCLES`]
    /// clears it, and a second load, a store that leaves `ACD` clear, or the bound passing
    /// refuses it, stamped at the load.
    #[cold]
    pub(super) fn gap_load(&mut self, core: &Core) {
        if let Some((_, refusal)) = self.gaps.acsr_read {
            self.gaps.refuse(refusal);
            return;
        }
        let refusal = Refusal {
            what: Refused::Setting {
                peripheral: "AC",
                setting: "aco_read",
            },
            at: self.oscillator_cycles(self.cycle),
            pc: core.pc(),
        };
        self.gaps.acsr_read = Some((self.cycle, refusal));
    }

    /// Refuses a load of `ACSR` whose bound passed by CPU cycle `now` with no store after it.
    #[inline(always)]
    pub(crate) fn gap_deadline(&mut self, now: u64) {
        if let Some((cycle, refusal)) = self.gaps.acsr_read
            && now >= cycle + ACSR_RMW_CYCLES
        {
            self.gaps.refuse(refusal);
        }
    }

    /// A firmware store of `value` to `addr`, after the device took it.
    #[cold]
    pub(super) fn gap_store(&mut self, core: &Core, addr: u16, value: u8) {
        if !self.gaps.on || self.gaps.refusal.is_some() {
            return;
        }
        let pc = core.pc();
        let cycle = self.cycle;
        let setting = |peripheral, setting| Refused::Setting {
            peripheral,
            setting,
        };
        match addr {
            ACSR => {
                if let Some((_, refusal)) = self.gaps.acsr_read.take()
                    && value & ACD == 0
                {
                    self.gaps.refuse(refusal);
                    return;
                }
                if value & ACD != 0 {
                    return;
                }
                // §23.3.2. `ACIE` raises the comparator interrupt, `ACIC` feeds Timer1's input
                // capture, and `ACBG` puts the bandgap on the positive input. Each acts on an
                // output the model works out from 0 V or VCC levels.
                let used = [(ACIE, "ACIE"), (ACIC, "ACIC"), (ACBG, "ACBG")]
                    .into_iter()
                    .find(|&(bit, _)| value & bit != 0);
                if let Some((_, name)) = used {
                    self.refuse(setting("AC", name), cycle, pc);
                }
            }
            ADCSRB => {
                // Table 24-6. `ADTS` 001 starts a conversion on the comparator's `ACI` edge.
                if value & 0x07 == 0x01 && self.comparator_on() {
                    self.refuse(setting("ADC", "ADTS"), cycle, pc);
                }
            }
            ADMUX => {
                // Table 24-3 marks `REFS` 10 reserved, and Table 24-4 marks `MUX` 1001 to 1101
                // reserved, so neither has a conversion to model.
                let mux = value & 0x0F;
                if value >> 6 == 0b10 {
                    self.refuse(setting("ADC", "REFS"), cycle, pc);
                } else if (0x09..=0x0D).contains(&mux) {
                    self.refuse(setting("ADC", "MUX"), cycle, pc);
                } else if mux == 6 || mux == 7 {
                    self.note(Unmodeled::AdcNoPad(mux), pc, true);
                }
            }
            MCUCR => {
                // §12.5.1. `IVSEL` moves the vectors to the boot section, and the model serves
                // them from word 0.
                if value & 0x02 != 0 {
                    self.refuse(setting("CPU", "IVSEL"), cycle, pc);
                }
            }
            SPMCSR => {
                // §27.9.1. Every `SPM` that acts follows a store here, since `SPMEN` enables
                // it, so this check covers `SPM` too. `SPMIE` enables the SPM ready interrupt.
                // The other bits pick what the next `SPM` or `LPM` does: fill the page buffer, program, erase, write the
                // lock bits, or read the signature, fuse or lock bits, and `SPMEN` clears
                // itself four cycles on. The model keeps the register as plain memory, so an
                // `LPM` after it reads flash and a poll of `SPMEN` never ends.
                if value & 0x80 != 0 {
                    self.refuse(
                        Refused::Interrupt {
                            peripheral: "BOOT_LOAD",
                        },
                        cycle,
                        pc,
                    );
                } else if value & 0x3F != 0 {
                    self.refuse(setting("BOOT_LOAD", "SPMCSR"), cycle, pc);
                }
            }
            OSCCAL => {
                // §9.12.1. `OSCCAL` tunes the RC oscillator, which times EEPROM writes
                // (§8.6.3). The system clock is the crystal, so the run is right but for the
                // EEPROM write time.
                self.note(Unmodeled::Osccal, pc, true);
            }
            ASSR => {
                // §18.11.8. `AS2` clocks Timer2 from a crystal on `TOSC1`, and `EXCLK` from an
                // external clock there. The model counts `clkI/O`.
                if value & 0x40 != 0 {
                    self.refuse(setting("TC2", "EXCLK"), cycle, pc);
                } else if value & 0x20 != 0 {
                    self.refuse(setting("TC2", "AS2"), cycle, pc);
                }
            }
            UCSR0C => {
                // Table 20-8. `UMSEL` 01 is synchronous mode, which clocks on `XCK`, and 10 is
                // reserved. Table 20-9 marks `UPM` 01 reserved.
                match value >> 6 {
                    0b01 => self.refuse(setting("USART0", "synchronous"), cycle, pc),
                    0b10 => self.refuse(setting("USART0", "UMSEL"), cycle, pc),
                    0b00 if value & 0x30 == 0x10 => {
                        self.refuse(setting("USART0", "UPM"), cycle, pc);
                    }
                    _ => {}
                }
            }
            TWAR => {
                self.gaps.twar_set = true;
                self.check_twi_slave(pc);
            }
            TWCR => {
                self.check_twi_slave(pc);
                self.check_twi_hold(value, pc);
            }
            _ => {}
        }
    }

    /// §22.7.3 and §22.7.4. With `TWEN` and `TWEA` set, the TWI answers its own `TWAR` address,
    /// and the general call when `TWGCE` is set, as a slave. The model has no slave modes, so
    /// nothing answers. Wire's master sets both bits in `TWCR` and leaves `TWAR` at its reset
    /// value, so the note asks for a firmware `TWAR` store too, as `Wire.begin(address)`
    /// makes. A run stays right until another master addresses the chip.
    fn check_twi_slave(&mut self, pc: u16) {
        const TWEA: u8 = 1 << 6;
        const TWEN: u8 = 1 << 2;
        if self.gaps.twar_set && self.io_byte(TWCR) & (TWEA | TWEN) == TWEA | TWEN {
            self.note(Unmodeled::TwiSlave, pc, true);
        }
    }

    /// A START at a bit rate whose data hold time in the model, `floor(T/16)` cycles with
    /// `T = 16 + 2·TWBR·4^TWPS` (§22.5.2), is past Table 29-14's 3.45 us `tHD;DAT` maximum. That
    /// is below about 18 kHz.
    fn check_twi_hold(&mut self, value: u8, pc: u16) {
        const TWSTA: u8 = 1 << 5;
        const TWEN: u8 = 1 << 2;
        if value & (TWSTA | TWEN) != TWSTA | TWEN {
            return;
        }
        let twps = self.io_byte(TWSR) & 0x03;
        let period = crate::periph::twi::scl_period(self.io_byte(TWBR), twps);
        let hold = u128::from(period / 16);
        if hold * 100_000_000 > 345 * u128::from(self.cpu_hz()) {
            self.note(Unmodeled::TwiHoldTime, pc, true);
        }
    }
}
