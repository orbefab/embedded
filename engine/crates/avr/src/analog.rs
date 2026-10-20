//! Host analog levels. The ADC samples these at its sample-and-hold cycle.
//!
//! Voltages are microvolts so `Vin × 1024 / Vref` is an integer division.
//! A floating input converts as 0 µV. That default is this model's choice:
//! the die leaves a floating channel undefined.

use crate::Level;

/// Nominal bandgap, §24.8 and Table 24-3. Typical, not a calibrated reference.
pub(crate) const BANDGAP_UV: i32 = 1_100_000;

/// Digital supply used for input thresholds.
/// DS40002061B Table 30-1, the `VCC = 2.4 V–5.5 V` row.
pub(crate) const VCC_UV: i32 = 5_000_000;

/// `VIL` max, Table 30-1: `0.3 VCC` at 5.0 V.
pub(crate) const VIL_UV: i32 = 1_500_000;

/// `VIH` min, Table 30-1: `0.6 VCC` at 5.0 V.
pub(crate) const VIH_UV: i32 = 3_000_000;

/// What the ADC or the comparator is measuring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sense {
    /// GPIO pin id (`Port::pin_id`).
    Pin(u8),
    /// ADC channel with no digital pad (ADC6/ADC7 on the 328P).
    Only(u8),
    /// ADC multiplexer channel. The bus maps it through the die's pin table.
    Mux(u8),
    Bandgap,
    /// §24.8 temperature sensor. Typical Table 24-2, not calibrated.
    Temp,
    Ground,
}

/// Last host action on one pin. The later call replaces the earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostPin {
    /// No host voltage and no host digital drive. The ADC reads 0.
    Float,
    /// `drive_pin` won. High is the 5.0 V rail; low and released read as 0.
    Level(Level),
    Volts(i32),
}

/// Voltages the host has applied, plus the Schmitt latch for each pin.
#[derive(Debug, Clone)]
pub(crate) struct HostAnalog {
    pin: [HostPin; crate::PIN_COUNT],
    /// Last decided digital level. Holds while the voltage is between the
    /// Table 30-1 thresholds.
    schmitt: [bool; crate::PIN_COUNT],
    /// Explicit voltage for a channel that has no GPIO pin.
    only: [Option<i32>; 16],
    avcc: i32,
    aref: i32,
    temp_milli: i32,
}

impl HostAnalog {
    pub(crate) fn new() -> Self {
        Self {
            pin: [HostPin::Float; crate::PIN_COUNT],
            schmitt: [false; crate::PIN_COUNT],
            only: [None; 16],
            avcc: VCC_UV,
            aref: VCC_UV,
            temp_milli: 25_000,
        }
    }

    pub(crate) fn set_avcc(&mut self, uv: i32) {
        self.avcc = uv;
    }

    pub(crate) fn set_aref(&mut self, uv: i32) {
        self.aref = uv;
    }

    pub(crate) fn set_temperature_milli(&mut self, milli: i32) {
        self.temp_milli = milli;
    }

    /// Record a host voltage and return the digital level the Schmitt trigger
    /// presents. `None` when `pin` is not a pad on this die.
    pub(crate) fn set_pin_voltage(&mut self, pin: u32, uv: i32) -> Option<Level> {
        let index = usize::try_from(pin).ok()?;
        if index >= self.pin.len() {
            return None;
        }
        let high = if uv <= VIL_UV {
            false
        } else if uv >= VIH_UV {
            true
        } else {
            self.schmitt[index]
        };
        self.schmitt[index] = high;
        self.pin[index] = HostPin::Volts(uv);
        Some(if high { Level::High } else { Level::Low })
    }

    /// `drive_pin` replaced any voltage. High reads as `VCC` to the ADC.
    pub(crate) fn set_pin_level(&mut self, pin: u32, level: Level) {
        let Ok(index) = usize::try_from(pin) else {
            return;
        };
        if index >= self.pin.len() {
            return;
        }
        self.schmitt[index] = level == Level::High;
        self.pin[index] = HostPin::Level(level);
    }

    pub(crate) fn set_only(&mut self, channel: u8, uv: i32) {
        if let Some(slot) = self.only.get_mut(usize::from(channel)) {
            *slot = Some(uv);
        }
    }

    pub(crate) fn resolve(&self, sense: Sense) -> i32 {
        match sense {
            Sense::Pin(pin) => self.pin_uv(pin),
            Sense::Only(channel) | Sense::Mux(channel) => self
                .only
                .get(usize::from(channel))
                .copied()
                .flatten()
                .unwrap_or(0),
            Sense::Bandgap => BANDGAP_UV,
            Sense::Temp => temperature_uv(self.temp_milli),
            Sense::Ground => 0,
        }
    }

    /// `REFS` from Table 24-3. `10` is reserved and uses AREF.
    pub(crate) fn vref(&self, refs: u8) -> i32 {
        match refs & 0x03 {
            0b01 => self.avcc,
            0b11 => BANDGAP_UV,
            _ => self.aref,
        }
    }

    fn pin_uv(&self, pin: u8) -> i32 {
        match self
            .pin
            .get(usize::from(pin))
            .copied()
            .unwrap_or(HostPin::Float)
        {
            HostPin::Volts(uv) => uv,
            HostPin::Level(Level::High) => VCC_UV,
            HostPin::Level(Level::Low) | HostPin::Level(Level::Released) | HostPin::Float => 0,
        }
    }
}

/// Nearest microvolt. Non-finite inputs become 0.
pub(crate) fn microvolts(volts: f32) -> i32 {
    if !volts.is_finite() {
        return 0;
    }
    let uv = (f64::from(volts) * 1_000_000.0).round();
    if uv >= f64::from(i32::MAX) {
        i32::MAX
    } else if uv <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        uv as i32
    }
}

/// Millidegrees Celsius from a host temperature.
pub(crate) fn millidegrees(celsius: f32) -> i32 {
    if !celsius.is_finite() {
        return 25_000;
    }
    let milli = (f64::from(celsius) * 1_000.0).round();
    if milli >= f64::from(i32::MAX) {
        i32::MAX
    } else if milli <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        milli as i32
    }
}

/// Table 24-2, piecewise linear through the three typical points.
///
/// Outside −45 °C…+85 °C the nearest segment is extended. This is the typical
/// column, not a calibrated `TOS`.
pub(crate) fn temperature_uv(milli_c: i32) -> i32 {
    const ROOM: i32 = 25_000;
    let (t_a, t_b, v_a, v_b) = if milli_c <= ROOM {
        (-45_000, ROOM, 242_000_i64, 314_000_i64)
    } else {
        (ROOM, 85_000, 314_000_i64, 380_000_i64)
    };
    let span = i64::from(t_b - t_a);
    let dv = v_b - v_a;
    (v_a + dv * i64::from(milli_c - t_a) / span) as i32
}

/// §24.7. `floor(Vin × 1024 / Vref)`, clamped to `0..=1023`.
///
/// `0x3FF` is `Vref` minus one LSB, so a `Vin` at or above `Vref` is 1023.
/// A non-positive input or reference is 0.
pub(crate) fn adc_code(vin: i32, vref: i32) -> u16 {
    if vin <= 0 || vref <= 0 {
        return 0;
    }
    let raw = i64::from(vin).saturating_mul(1024) / i64::from(vref);
    if raw > 1023 { 1023 } else { raw as u16 }
}
