//! Analog comparator. DS40002061B §23.
//!
//! The output is compared only when an input voltage or a selection changes.
//! `ACO` then updates `ACO_DELAY` CPU clocks later (§23.3.2: the synchronizer
//! is 1–2 clocks; this model uses 2, the stated maximum). `ACI` follows that
//! synchronized edge. `ACIC` tells the bus to feed Timer1's capture front-end
//! instead of `ICP1`.

use crate::analog::Sense;
use crate::mux::PinMux;
use crate::periph::{Ctx, Notice, Touch};
use crate::regs::RegBlock;

pub(crate) const REG_ACSR: u8 = 0;
pub(crate) const REG_DIDR1: u8 = 1;

const ACD: u8 = 1 << 7;
const ACBG: u8 = 1 << 6;
const ACO: u8 = 1 << 5;
const ACI: u8 = 1 << 4;
const ACIE: u8 = 1 << 3;
const ACIC: u8 = 1 << 2;

/// §23.3.2. First cycle past the 1–2 clock synchronizer.
pub(crate) const ACO_DELAY: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcmpCfg {
    pub vec: u8,
    pub ain0: u8,
    pub ain1: u8,
    pub regs: RegBlock,
}

/// What the bus should compare. `disabled` forces the raw output low.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Compare {
    pub disabled: bool,
    pub pos: Sense,
    pub neg: Sense,
}

#[derive(Debug, Clone)]
pub(crate) struct Acmp {
    vec: u8,
    ain0: u8,
    ain1: u8,
    acsr: u8,
    didr1: u8,
    /// Multiplexer state copied from the ADC. `mux` is `MUX3:0`.
    acme: bool,
    aden: bool,
    mux: u8,
    /// Unsynchronized comparison.
    raw: bool,
    /// Synchronized `ACO`.
    aco: bool,
    aco_due: u64,
    pending: bool,
    aci: bool,
    next: u64,
    notice: Notice,
}

impl Acmp {
    pub(crate) fn new(cfg: &AcmpCfg) -> Self {
        Self {
            vec: cfg.vec,
            ain0: cfg.ain0,
            ain1: cfg.ain1,
            acsr: 0,
            didr1: 0,
            acme: false,
            aden: false,
            mux: 0,
            raw: false,
            aco: false,
            aco_due: u64::MAX,
            pending: false,
            aci: false,
            next: u64::MAX,
            notice: Notice::default(),
        }
    }

    /// Synchronized `ACO`. The bus copies this when `ACIC` is enabled.
    pub(crate) fn output_high(&self) -> bool {
        self.aco
    }

    pub(crate) fn compare(&self) -> Compare {
        Compare {
            disabled: self.acsr & ACD != 0,
            pos: if self.acsr & ACBG != 0 {
                Sense::Bandgap
            } else {
                Sense::Pin(self.ain0)
            },
            neg: if self.acme && !self.aden {
                // Table 23-1. `MUX2:0` replaces AIN1 with ADC0–ADC7.
                Sense::Mux(self.mux & 0x07)
            } else {
                Sense::Pin(self.ain1)
            },
        }
    }

    /// ADC register write. Returns whether the selected inputs changed.
    pub(crate) fn set_mux(&mut self, acme: bool, aden: bool, channel: u8) -> bool {
        let changed = acme != self.acme || aden != self.aden || channel != self.mux;
        self.acme = acme;
        self.aden = aden;
        self.mux = channel;
        changed
    }

    /// `high` is `V+ > V−`. Equal stays low. A change is visible at `at + 2`.
    pub(crate) fn consider(&mut self, mut high: bool, at: u64) -> bool {
        if self.acsr & ACD != 0 {
            high = false;
        }
        if high == self.raw {
            return false;
        }
        self.raw = high;
        self.pending = high;
        if high == self.aco {
            self.aco_due = u64::MAX;
            self.next = u64::MAX;
            return true;
        }
        self.aco_due = at.saturating_add(ACO_DELAY);
        self.next = self.aco_due;
        true
    }

    pub(crate) fn read(&mut self, reg: u8) -> u8 {
        self.peek(reg)
    }

    pub(crate) fn peek(&self, reg: u8) -> u8 {
        match reg {
            REG_ACSR => {
                let mut value = self.acsr & !(ACO | ACI);
                if self.aco {
                    value |= ACO;
                }
                if self.aci {
                    value |= ACI;
                }
                value
            }
            REG_DIDR1 => self.didr1,
            _ => 0,
        }
    }

    pub(crate) fn write(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
    ) -> Touch {
        self.apply(reg, value, mux, ctx, cycle)
    }

    pub(crate) fn irq_bits(&self) -> u64 {
        if self.aci && self.acsr & ACIE != 0 {
            1u64 << self.vec
        } else {
            0
        }
    }

    pub(crate) fn ack(&mut self, vector: u8) {
        if vector == self.vec {
            self.lower_aci();
        }
    }

    pub(crate) fn next_event(&self) -> u64 {
        self.next
    }

    /// `ACO` or `ACI` may have changed. The bus forwards the notice.
    pub(crate) fn fire(&mut self, at: u64) {
        if self.aco_due != at {
            return;
        }
        self.aco_due = u64::MAX;
        self.next = u64::MAX;
        let next = self.pending;
        if next == self.aco {
            return;
        }
        let was = self.aco;
        self.aco = next;
        self.notice.aco = true;
        self.notice.aco_high = next;
        let mode = self.acsr & 0x03;
        let trip = match mode {
            0 => true,
            0b10 => was && !next,
            0b11 => !was && next,
            _ => false,
        };
        if trip {
            self.raise_aci();
        }
    }

    pub(crate) fn take_notice(&mut self) -> Notice {
        let notice = self.notice;
        self.notice = Notice::default();
        notice
    }

    fn apply(
        &mut self,
        reg: u8,
        value: u8,
        mux: &mut PinMux,
        ctx: &mut Ctx<'_>,
        cycle: u64,
    ) -> Touch {
        match reg {
            REG_ACSR => {
                if value & ACI != 0 {
                    self.lower_aci();
                }
                let acic = value & ACIC != 0;
                if acic != (self.acsr & ACIC != 0) {
                    self.notice.acic = true;
                    self.notice.acic_on = acic;
                }
                // `ACO` is read-only. `ACI` is write-one-to-clear.
                self.acsr = value & !(ACO | ACI);
                self.notice.refresh = true;
                Touch {
                    irq: true,
                    schedule: true,
                    ..Touch::default()
                }
            }
            REG_DIDR1 => {
                self.didr1 = value & 0x03;
                mux.set_buffer_disabled(self.ain0, self.didr1 & 0x01 != 0, ctx, cycle);
                mux.set_buffer_disabled(self.ain1, self.didr1 & 0x02 != 0, ctx, cycle);
                Touch::default()
            }
            _ => Touch::default(),
        }
    }

    fn raise_aci(&mut self) {
        if !self.aci {
            self.aci = true;
            self.notice.rose |= super::adc::TRIG_ACI;
        }
    }

    fn lower_aci(&mut self) {
        if self.aci {
            self.aci = false;
            self.notice.fell |= super::adc::TRIG_ACI;
        }
    }
}
