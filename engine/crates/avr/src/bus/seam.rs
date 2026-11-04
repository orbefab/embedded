//! What a host that runs the core on its own clock reads and answers: the ADC samples it is
//! asked for, whether a step left pin reports, and what it drives on the pads.

use super::Bus;
use crate::Level;
use crate::analog::{Sense, adc_code};

/// Where the ADC's sample-and-hold reads its volts.
#[derive(Debug, Clone, Copy)]
pub(super) enum Samples {
    /// The host's voltage store, at the cycle.
    Store,
    /// The host, asked at the cycle. `due` is a sample-and-hold it has not answered.
    Asked { due: Option<u64> },
}

impl Bus {
    /// Ask the host for each sample-and-hold's volts instead of reading the store.
    pub(crate) fn ask_for_samples(&mut self) {
        self.samples = Samples::Asked { due: None };
    }

    pub(crate) fn asks_for_samples(&self) -> bool {
        matches!(self.samples, Samples::Asked { .. })
    }

    pub(crate) fn sample_due(&self) -> Option<u64> {
        match self.samples {
            Samples::Asked { due } => due,
            Samples::Store => None,
        }
    }

    /// The volts at the waiting sample-and-hold, in microvolts: ADC0 to ADC7, `AVCC`, and
    /// `AREF`. The code is published at completion, as a stored sample is.
    pub(crate) fn answer_sample(&mut self, channels: [i32; 8], avcc: i32, aref: i32) {
        let Samples::Asked { due: Some(_) } = self.samples else {
            return;
        };
        self.samples = Samples::Asked { due: None };
        let Some((sense, refs)) = self.adc().map(|adc| adc.latched_sense()) else {
            return;
        };
        let channel_of = |pin: u8| {
            (0..8u8)
                .find(|&channel| self.adc().and_then(|adc| adc.channel_pin(channel)) == Some(pin))
        };
        let vin = match sense {
            Sense::Pin(pin) => channel_of(pin).map_or(0, |channel| channels[usize::from(channel)]),
            Sense::Only(channel) | Sense::Mux(channel) => {
                channels.get(usize::from(channel)).copied().unwrap_or(0)
            }
            other => self.analog.resolve(other),
        };
        let vref = match refs & 0x03 {
            0b01 => avcc,
            0b11 => crate::analog::BANDGAP_UV,
            _ => aref,
        };
        let code = adc_code(vin, vref);
        if let Some(adc) = self.adc_mut() {
            adc.hold(code);
        }
    }

    pub(crate) fn has_pin_reports(&self) -> bool {
        !self.host.pins.is_empty()
    }

    pub(crate) fn host_levels(&self) -> [Level; crate::PIN_COUNT] {
        self.mux.host_levels()
    }

    pub(crate) fn is_reset_pin(&self, pin: u32) -> bool {
        self.mux.is_reset_pin(pin)
    }
}
