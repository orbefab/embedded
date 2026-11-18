//! GPIO, IO_MUX and the GPIO matrix: which level each pad drives, what `GPIO_IN` reads, the
//! level an input signal reads, and the pads' interrupts.
//!
//! A pad drives from the matrix when its IO_MUX function is the GPIO one: function 1 on every
//! pad, or function 0 on a pad whose function 0 is its GPIO. The matrix's signal 256 is the
//! pad's own `GPIO_OUT` bit, with `GPIO_ENABLE` as its output enable. A peripheral signal the
//! SoC gives a level, as SPI2 does its idle levels and RMT does signal 81, drives that level, enabled by the peripheral
//! unless `OEN_SEL` hands the enable to `GPIO_ENABLE`. A pad routed to any other signal, or to
//! another IO_MUX function, drives nothing. A pad whose `GPIO_PINn` sets `PAD_DRIVER` is open
//! drain, and drives nothing for a high level. A pad that does not drive pulls up when `FUN_WPU`
//! is set, pulls down when `FUN_WPD` is set, and pulls both ways when both are, through both
//! resistors to their rails.
//!
//! Each change that can move a level senses the pads, and latches each edge its pad's
//! `INT_TYPE` names into `GPIO_STATUS`. A level type is pending while its level holds.

/// Pads GPIO0 to GPIO48. GPIO22 to GPIO25 have no pad, and their bits do nothing.
pub(crate) const PADS: usize = 49;

/// The bits of the 54 the registers hold.
const BITS: u64 = (1 << 54) - 1;

/// The output signals of the matrix, 0 to 255. 256 is the pad's own `GPIO_OUT` bit.
const SIGNALS: usize = 256;

/// `FUNCn_OUT_SEL_CFG`'s value for the pad's own `GPIO_OUT` bit.
const SIG_GPIO_OUT: u32 = 0x100;
const OUT_SEL: u32 = 0x1ff;
const OUT_INV_SEL: u32 = 1 << 9;
const OEN_SEL: u32 = 1 << 10;
const OEN_INV_SEL: u32 = 1 << 11;

/// `FUNCn_IN_SEL_CFG`'s fields, and the two `IN_SEL` values that read a constant.
const IN_SEL: u32 = 0x3f;
const IN_INV_SEL: u32 = 1 << 6;
const SIG_IN_SEL: u32 = 1 << 7;
const IN_HIGH: u32 = 0x38;
const IN_LOW: u32 = 0x3c;

/// `GPIO_PINn`'s fields.
const PAD_DRIVER: u32 = 1 << 2;
const INT_TYPE_SHIFT: u32 = 7;
const WAKEUP_ENABLE: u32 = 1 << 10;
const INT_ENA_SHIFT: u32 = 13;
const PIN_BITS: u32 = (1 << 18) - 1;
/// `SYNC2_BYPASS`, `SYNC1_BYPASS` and `CONFIG`, kept and not run. The model reads a pad's level
/// at the tick it changes, through no synchronizer.
pub(crate) const PIN_UNRUN: u32 = 0x3 | 0x3 << 3 | 0x3 << 11;

const FUN_WPD: u32 = 1 << 7;
const FUN_WPU: u32 = 1 << 8;
const FUN_IE: u32 = 1 << 9;
const MCU_SEL_SHIFT: u32 = 12;
pub(crate) const MCU_SEL: u32 = 0x7 << MCU_SEL_SHIFT;
/// `FUN_DRV` 2, no pull, input off and function 0, as every pad reads at power-on here. The
/// datasheet's power-on pulls and input enables are not modeled.
const IO_MUX_RESET: u32 = 0x800;

/// The pads whose IO_MUX function 0 is their GPIO, from IDF's `io_mux_reg.h`: 0 to 21, 33 to
/// 38, 45 and 46. Function 0 of the others is flash SPI, JTAG, UART0 or the SPI clock pair.
const GPIO_FUNCTION_0: u64 = 0x3fffff | (0x3f << 33) | (0x3 << 45);

/// What a pad does to its net.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadDrive {
    High,
    Low,
    PullUp,
    PullDown,
    /// Both pulls, each to its rail.
    PullBoth,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Gpio {
    out: u64,
    enable: u64,
    out_sel: [u32; 54],
    in_sel: [u32; 256],
    io_mux: [u32; PADS],
    /// The level each pad with a net reads, `None` for one with no net.
    pads: [Option<bool>; PADS],
    /// The level of each peripheral output signal the SoC drives, `None` for one it does not.
    signals: [Option<bool>; SIGNALS],
    /// `GPIO_PIN0` to `GPIO_PIN48`, as stored.
    pin: [u32; PADS],
    /// The pads whose `GPIO_PINn` sets `INT_ENA` bit 0.
    enabled: u64,
    /// `GPIO_STATUS` and `GPIO_STATUS1` as one word, the latched edges.
    status: u64,
    /// `GPIO_IN` and `GPIO_IN1` as one word, as the last change left them.
    levels: u64,
}

/// `GPIO_PINn`'s `INT_TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    Off,
    Rise,
    Fall,
    Any,
    Low,
    High,
}

impl Trigger {
    /// The type `GPIO_PINn` word `pin` names, `None` for the reserved 6 and 7.
    fn of(pin: u32) -> Option<Trigger> {
        Some(match pin >> INT_TYPE_SHIFT & 0x7 {
            0 => Trigger::Off,
            1 => Trigger::Rise,
            2 => Trigger::Fall,
            3 => Trigger::Any,
            4 => Trigger::Low,
            5 => Trigger::High,
            _ => return None,
        })
    }
}

/// A GPIO block register the model keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reg {
    Out(Word),
    Enable(Word),
    In(usize),
    /// `GPIO_STATUS` and `GPIO_STATUS1`, and their `W1TC` words.
    Status(Word),
    /// `GPIO_PCPU_INT` and `GPIO_PCPU_INT1`, which both cores read.
    Pending(usize),
    Pin(usize),
    OutSel(usize),
    InSel(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Set(usize),
    W1ts(usize),
    W1tc(usize),
}

impl Gpio {
    /// The pads listed in `netted` have nets, and read low until the first input.
    pub(crate) fn new(netted: &[u8]) -> Gpio {
        let mut pads = [None; PADS];
        for &gpio in netted {
            pads[usize::from(gpio)] = Some(false);
        }
        let mut gpio = Gpio {
            out: 0,
            enable: 0,
            out_sel: [SIG_GPIO_OUT; 54],
            in_sel: [0; 256],
            io_mux: [IO_MUX_RESET; PADS],
            pads,
            signals: [None; SIGNALS],
            pin: [0; PADS],
            enabled: 0,
            status: 0,
            levels: 0,
        };
        gpio.levels = gpio.sample();
        gpio
    }

    /// A register at `offset` in the GPIO block, or `None` for one the model does not keep.
    fn reg(offset: u32) -> Option<Reg> {
        let word = |base: u32, half: usize| match offset - base {
            0 => Word::Set(half),
            4 => Word::W1ts(half),
            8 => Word::W1tc(half),
            _ => unreachable!(),
        };
        Some(match offset {
            0x04..=0x0c => Reg::Out(word(0x04, 0)),
            0x10..=0x18 => Reg::Out(word(0x10, 1)),
            0x20..=0x28 => Reg::Enable(word(0x20, 0)),
            0x2c..=0x34 => Reg::Enable(word(0x2c, 1)),
            0x3c => Reg::In(0),
            0x40 => Reg::In(1),
            0x44 => Reg::Status(Word::Set(0)),
            0x4c => Reg::Status(Word::W1tc(0)),
            0x50 => Reg::Status(Word::Set(1)),
            0x58 => Reg::Status(Word::W1tc(1)),
            0x5c => Reg::Pending(0),
            0x68 => Reg::Pending(1),
            0x74..=0x134 => Reg::Pin(((offset - 0x74) / 4) as usize),
            0x154..=0x550 => Reg::InSel(((offset - 0x154) / 4) as usize),
            0x554..=0x628 => Reg::OutSel(((offset - 0x554) / 4) as usize),
            _ => return None,
        })
    }

    /// The GPIO block register at `offset`, a multiple of 4, or `None` for one not kept.
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        let half = |bits: u64, half: usize| (bits >> (32 * half)) as u32;
        Some(match Self::reg(offset)? {
            Reg::Out(Word::Set(k)) => half(self.out, k),
            Reg::Enable(Word::Set(k)) => half(self.enable, k),
            Reg::Status(Word::Set(k)) => half(self.status | self.held((1 << PADS) - 1), k),
            Reg::Out(_) | Reg::Enable(_) | Reg::Status(_) => 0,
            Reg::In(k) => half(self.levels, k),
            Reg::Pending(k) => half(self.pending(), k),
            Reg::Pin(n) => self.pin[n],
            Reg::OutSel(n) => self.out_sel[n],
            Reg::InSel(n) => self.in_sel[n],
        })
    }

    /// Writes the register at `offset`. `Ok(false)` for one not kept, which drops the write, and
    /// `Err` with the setting's name, writing nothing, for a `GPIO_PINn` word that sets an
    /// interrupt the model does not run.
    pub(crate) fn write(&mut self, offset: u32, value: u32) -> Result<bool, &'static str> {
        let Some(reg) = Self::reg(offset) else {
            return Ok(false);
        };
        let apply = |bits: &mut u64, word: Word| {
            let (k, op) = match word {
                Word::Set(k) => (k, 0),
                Word::W1ts(k) => (k, 1),
                Word::W1tc(k) => (k, 2),
            };
            let shift = 32 * k;
            let mask = (u64::from(u32::MAX) << shift) & BITS;
            let value = (u64::from(value) << shift) & mask;
            *bits = match op {
                0 => (*bits & !mask) | value,
                1 => *bits | value,
                _ => *bits & !value,
            };
        };
        match reg {
            Reg::Out(word) => apply(&mut self.out, word),
            Reg::Enable(word) => apply(&mut self.enable, word),
            Reg::Status(word) => apply(&mut self.status, word),
            Reg::In(_) | Reg::Pending(_) => {}
            Reg::Pin(n) => {
                if value >> INT_ENA_SHIFT & 0x1e != 0 {
                    return Err("pin_int_ena");
                }
                if Trigger::of(value).is_none() {
                    return Err("pin_int_type");
                }
                if value & WAKEUP_ENABLE != 0 {
                    return Err("pin_wakeup");
                }
                self.pin[n] = value & PIN_BITS;
                let bit = 1 << n;
                self.enabled = (self.enabled & !bit) | u64::from(value >> INT_ENA_SHIFT & 1) << n;
            }
            Reg::OutSel(n) => self.out_sel[n] = value & 0xfff,
            Reg::InSel(n) => self.in_sel[n] = value & 0xff,
        }
        self.sense();
        Ok(true)
    }

    /// The IO_MUX register at `offset` in its block, a pad's for 0x04 to 0xc4.
    pub(crate) fn read_io_mux(&self, offset: u32) -> Option<u32> {
        Some(self.io_mux[Self::io_mux_pad(offset)?])
    }

    pub(crate) fn write_io_mux(&mut self, offset: u32, value: u32) -> bool {
        let Some(pad) = Self::io_mux_pad(offset) else {
            return false;
        };
        self.io_mux[pad] = value & 0xffff;
        self.sense();
        true
    }

    fn io_mux_pad(offset: u32) -> Option<usize> {
        let pad = (offset.checked_sub(4)? / 4) as usize;
        (pad < PADS).then_some(pad)
    }

    /// The level a pad with a net reads from now on.
    pub(crate) fn set_pad(&mut self, gpio: u8, level: bool) {
        let pad = &mut self.pads[usize::from(gpio)];
        debug_assert!(pad.is_some(), "GPIO{gpio} has a net");
        *pad = Some(level);
        self.sense();
    }

    /// The level peripheral output `signal` drives from now on.
    pub(crate) fn set_signal(&mut self, signal: u16, level: bool) {
        let held = &mut self.signals[usize::from(signal)];
        if *held != Some(level) {
            *held = Some(level);
            self.sense();
        }
    }

    /// Whether `signal` at `level` would move the drive of a pad with a net.
    pub(crate) fn moves(&self, signal: u16, level: bool) -> bool {
        self.signals[usize::from(signal)] != Some(level)
            && (0..PADS).any(|pad| {
                self.pads[pad].is_some() && self.carries(pad).is_some_and(|(s, _)| s == signal)
            })
    }

    /// The level input `signal` reads through the matrix, or `None` when `SIG_IN_SEL` leaves it
    /// to IO_MUX or `IN_SEL` names no pad.
    pub(crate) fn input(&self, signal: u16) -> Option<bool> {
        let sel = self.in_sel[usize::from(signal)];
        if sel & SIG_IN_SEL == 0 {
            return None;
        }
        let level = match sel & IN_SEL {
            IN_HIGH => true,
            IN_LOW => false,
            pad if (pad as usize) < PADS => self.levels >> pad & 1 != 0,
            _ => return None,
        };
        Some(level ^ (sel & IN_INV_SEL != 0))
    }

    /// The pad input `signal` reads through the matrix, uninverted.
    pub(crate) fn reads(&self, signal: u16) -> Option<u8> {
        let sel = self.in_sel[usize::from(signal)];
        let pad = (sel & IN_SEL) as usize;
        (sel & (SIG_IN_SEL | IN_INV_SEL) == SIG_IN_SEL && pad < PADS).then_some(pad as u8)
    }

    /// Whether the pad's `GPIO_PINn` sets `PAD_DRIVER`.
    pub(crate) fn open_drain(&self, pad: u8) -> bool {
        self.pin[usize::from(pad)] & PAD_DRIVER != 0
    }

    /// The lowest pad whose output `signal` drives, enabled.
    pub(crate) fn routed(&self, signal: u16) -> Option<u8> {
        (0..PADS)
            .find(|&pad| self.out_sel[pad] & OUT_SEL == u32::from(signal) && self.output(pad).0)
            .map(|pad| pad as u8)
    }

    /// The peripheral signal the pad drives from the matrix, enabled, and whether `OUT_INV_SEL`
    /// inverts it.
    pub(crate) fn carries(&self, pad: usize) -> Option<(u16, bool)> {
        let sel = self.out_sel[pad];
        let signal = (sel & OUT_SEL) as u16;
        let peripheral = usize::from(signal) < SIGNALS && self.output(pad).0;
        peripheral.then_some((signal, sel & OUT_INV_SEL != 0))
    }

    /// Whether the pad takes its output from the GPIO matrix.
    fn matrix_drives(&self, pad: usize) -> bool {
        match (self.io_mux[pad] & MCU_SEL) >> MCU_SEL_SHIFT {
            1 => true,
            0 => GPIO_FUNCTION_0 >> pad & 1 != 0,
            _ => false,
        }
    }

    /// The output enable and the level the pad drives when enabled.
    fn output(&self, pad: usize) -> (bool, bool) {
        let sel = self.out_sel[pad];
        if !self.matrix_drives(pad) {
            return (false, false);
        }
        let gpio_enable =
            (self.enable >> pad & 1 != 0) ^ (sel & OEN_SEL != 0 && sel & OEN_INV_SEL != 0);
        let (enable, level) = match (sel & OUT_SEL) as usize {
            signal if signal == SIG_GPIO_OUT as usize => (gpio_enable, self.out >> pad & 1 != 0),
            signal => match self.signals.get(signal).copied().flatten() {
                Some(level) => (sel & OEN_SEL == 0 || gpio_enable, level),
                None => return (false, false),
            },
        };
        (enable, level ^ (sel & OUT_INV_SEL != 0))
    }

    pub(crate) fn drive(&self, pad: usize) -> PadDrive {
        let open_drain = self.pin[pad] & PAD_DRIVER != 0;
        match self.output(pad) {
            (true, true) if !open_drain => PadDrive::High,
            (true, false) => PadDrive::Low,
            _ => match (
                self.io_mux[pad] & FUN_WPU != 0,
                self.io_mux[pad] & FUN_WPD != 0,
            ) {
                (true, true) => PadDrive::PullBoth,
                (true, false) => PadDrive::PullUp,
                (false, true) => PadDrive::PullDown,
                (false, false) => PadDrive::Off,
            },
        }
    }

    /// `GPIO_IN` and `GPIO_IN1` as one word. A pad with its input disabled reads 0. One with a
    /// net reads its net. One with no net reads what it drives, else its pull. With both pulls
    /// it keeps the level it read last, since two equal pulls hold it at half the supply,
    /// between the input thresholds.
    fn sample(&self) -> u64 {
        (0..PADS)
            .filter(|&pad| self.io_mux[pad] & FUN_IE != 0)
            .filter(|&pad| match self.pads[pad] {
                Some(level) => level,
                None => match self.drive(pad) {
                    PadDrive::High | PadDrive::PullUp => true,
                    PadDrive::PullBoth => self.levels >> pad & 1 != 0,
                    PadDrive::Low | PadDrive::PullDown | PadDrive::Off => false,
                },
            })
            .fold(0, |bits, pad| bits | 1 << pad)
    }

    /// Takes the levels as they now are, and latches each edge whose pad's type takes it.
    fn sense(&mut self) {
        let levels = self.sample();
        let (rose, fell) = (levels & !self.levels, self.levels & !levels);
        self.levels = levels;
        let mut moved = rose | fell;
        while moved != 0 {
            let pad = moved.trailing_zeros() as usize;
            moved &= moved - 1;
            let bit = 1 << pad;
            let latched = match Trigger::of(self.pin[pad]) {
                Some(Trigger::Rise) => rose & bit,
                Some(Trigger::Fall) => fell & bit,
                Some(Trigger::Any) => bit,
                _ => 0,
            };
            self.status |= latched;
        }
    }

    /// The pads of `pads` whose level type matches their level.
    fn held(&self, mut pads: u64) -> u64 {
        let mut held = 0;
        while pads != 0 {
            let pad = pads.trailing_zeros() as usize;
            pads &= pads - 1;
            let high = self.levels >> pad & 1 != 0;
            let holds = match Trigger::of(self.pin[pad]) {
                Some(Trigger::Low) => !high,
                Some(Trigger::High) => high,
                _ => false,
            };
            held |= u64::from(holds) << pad;
        }
        held
    }

    /// `GPIO_PCPU_INT` and `GPIO_PCPU_INT1` as one word: the latched edges and the held levels
    /// of the pads whose interrupt is enabled.
    pub(crate) fn pending(&self) -> u64 {
        if self.enabled == 0 {
            return 0;
        }
        (self.status | self.held(self.enabled)) & self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPIO_FUNC: u32 = 1 << MCU_SEL_SHIFT;

    fn gpio21() -> Gpio {
        let mut gpio = Gpio::new(&[21]);
        gpio.write_io_mux(0x4 + 4 * 21, GPIO_FUNC | FUN_IE);
        gpio
    }

    #[test]
    fn a_pad_drives_its_out_bit_once_enabled() {
        let mut gpio = gpio21();
        assert_eq!(gpio.drive(21), PadDrive::Off);
        gpio.write(0x08, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::Off);
        gpio.write(0x24, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::High);
        gpio.write(0x0c, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::Low);
        assert_eq!(gpio.read(0x04), Some(0));
        assert_eq!(gpio.read(0x20), Some(1 << 21));
    }

    #[test]
    fn the_high_word_reaches_gpio32_and_up() {
        let mut gpio = Gpio::new(&[]);
        gpio.write_io_mux(0x4 + 4 * 40, GPIO_FUNC);
        gpio.write(0x30, 1 << 8).unwrap();
        gpio.write(0x14, 1 << 8).unwrap();
        assert_eq!(gpio.drive(40), PadDrive::High);
        assert_eq!(gpio.read(0x10), Some(1 << 8));
    }

    #[test]
    fn a_pad_reads_its_net_and_one_with_no_net_reads_itself() {
        let mut gpio = gpio21();
        gpio.set_pad(21, true);
        assert_eq!(gpio.read(0x3c), Some(1 << 21));
        gpio.write_io_mux(0x4 + 4 * 21, GPIO_FUNC);
        assert_eq!(gpio.read(0x3c), Some(0), "input disabled");

        gpio.write_io_mux(0x4 + 4 * 5, GPIO_FUNC | FUN_IE | FUN_WPU);
        assert_eq!(gpio.read(0x3c), Some(1 << 5), "pulled up");
        assert_eq!(gpio.drive(5), PadDrive::PullUp);
        gpio.write(0x24, 1 << 5).unwrap();
        assert_eq!(gpio.read(0x3c), Some(0), "drives its own low");
    }

    #[test]
    fn another_function_or_signal_drives_nothing() {
        let mut gpio = Gpio::new(&[]);
        gpio.write(0x30, 1 << (43 - 32)).unwrap();
        gpio.write(0x14, 1 << (43 - 32)).unwrap();
        assert_eq!(
            gpio.drive(43),
            PadDrive::Off,
            "function 0 of GPIO43 is U0TXD"
        );
        gpio.write_io_mux(0x4 + 4 * 43, GPIO_FUNC);
        assert_eq!(gpio.drive(43), PadDrive::High);
        gpio.write(0x554 + 4 * 43, 12).unwrap();
        assert_eq!(gpio.drive(43), PadDrive::Off, "a peripheral signal");
        gpio.write(0x554 + 4 * 43, 12 | OEN_SEL).unwrap();
        assert_eq!(
            gpio.drive(43),
            PadDrive::Off,
            "a peripheral signal, enabled from GPIO_ENABLE"
        );
        gpio.write(0x554 + 4 * 43, SIG_GPIO_OUT | OUT_INV_SEL)
            .unwrap();
        assert_eq!(gpio.drive(43), PadDrive::Low, "inverted");
    }

    #[test]
    fn a_peripheral_signal_drives_its_level_enabled_by_the_peripheral() {
        let mut gpio = Gpio::new(&[]);
        gpio.write_io_mux(0x4 + 4 * 12, GPIO_FUNC);
        gpio.write(0x554 + 4 * 12, 110).unwrap();
        assert_eq!(gpio.drive(12), PadDrive::Off, "a signal with no level");
        assert_eq!(gpio.routed(110), None);
        gpio.set_signal(110, true);
        assert_eq!(gpio.drive(12), PadDrive::High, "GPIO_ENABLE clear");
        gpio.write(0x554 + 4 * 12, 110 | OUT_INV_SEL).unwrap();
        assert_eq!(gpio.drive(12), PadDrive::Low, "inverted");
        gpio.write(0x554 + 4 * 12, 110 | OEN_SEL).unwrap();
        assert_eq!(gpio.drive(12), PadDrive::Off, "enabled from GPIO_ENABLE");
        assert_eq!(
            gpio.routed(110),
            None,
            "a pad that does not drive carries nothing"
        );
        gpio.write(0x24, 1 << 12).unwrap();
        assert_eq!(gpio.drive(12), PadDrive::High);
        gpio.write_io_mux(0x4 + 4 * 30, GPIO_FUNC);
        gpio.write(0x554 + 4 * 30, 110).unwrap();
        assert_eq!(gpio.routed(110), Some(12), "the lowest pad");
    }

    #[test]
    fn a_signal_change_moves_the_level_of_a_pad_with_no_net() {
        let mut gpio = Gpio::new(&[]);
        gpio.write_io_mux(0x4 + 4 * 12, GPIO_FUNC | FUN_IE);
        gpio.write(0x554 + 4 * 12, 110).unwrap();
        gpio.write(0x74 + 4 * 12, 1 << INT_TYPE_SHIFT).unwrap();
        gpio.set_signal(110, true);
        assert_eq!(gpio.read(0x3c), Some(1 << 12));
        assert_eq!(gpio.read(0x44), Some(1 << 12), "a rise");
        gpio.set_signal(110, false);
        assert_eq!(gpio.read(0x3c), Some(0));
    }

    #[test]
    fn oen_inv_sel_inverts_gpio_enable_only_under_oen_sel() {
        let mut gpio = Gpio::new(&[]);
        gpio.write_io_mux(0x4 + 4 * 43, GPIO_FUNC);
        gpio.write(0x14, 1 << (43 - 32)).unwrap();
        gpio.write(0x554 + 4 * 43, SIG_GPIO_OUT | OEN_INV_SEL)
            .unwrap();
        assert_eq!(gpio.drive(43), PadDrive::Off, "GPIO_ENABLE clear");
        gpio.write(0x554 + 4 * 43, SIG_GPIO_OUT | OEN_SEL | OEN_INV_SEL)
            .unwrap();
        assert_eq!(
            gpio.drive(43),
            PadDrive::High,
            "GPIO_ENABLE clear, inverted"
        );
        gpio.write(0x30, 1 << (43 - 32)).unwrap();
        assert_eq!(gpio.drive(43), PadDrive::Off, "GPIO_ENABLE set, inverted");
    }

    #[test]
    fn a_pad_that_does_not_drive_pulls_down_up_or_both_ways() {
        let mut gpio = gpio21();
        let mux = |pulls: u32| (0x4 + 4 * 21, GPIO_FUNC | FUN_IE | pulls);
        let (offset, value) = mux(FUN_WPD);
        gpio.write_io_mux(offset, value);
        assert_eq!(gpio.drive(21), PadDrive::PullDown);
        let (offset, value) = mux(FUN_WPU | FUN_WPD);
        gpio.write_io_mux(offset, value);
        assert_eq!(gpio.drive(21), PadDrive::PullBoth);
        let (offset, value) = mux(FUN_WPU);
        gpio.write_io_mux(offset, value);
        assert_eq!(gpio.drive(21), PadDrive::PullUp);
        let (offset, value) = mux(FUN_WPD);
        gpio.write_io_mux(offset, value);
        gpio.write(0x24, 1 << 21).unwrap();
        gpio.write(0x08, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::High, "a driven pad does not pull");
        gpio.write(0x0c, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::Low);
        gpio.write(0x74 + 4 * 21, PAD_DRIVER).unwrap();
        gpio.write(0x08, 1 << 21).unwrap();
        assert_eq!(
            gpio.drive(21),
            PadDrive::PullDown,
            "an open-drain high pulls down"
        );
    }

    #[test]
    fn a_pad_with_no_net_reads_its_pull_down_and_keeps_its_level_under_both() {
        let mut gpio = Gpio::new(&[]);
        let mux = |pulls: u32| GPIO_FUNC | FUN_IE | pulls;
        gpio.write_io_mux(0x4 + 4 * 5, mux(FUN_WPU));
        assert_eq!(gpio.read(0x3c).unwrap() & 1 << 5, 1 << 5);
        gpio.write_io_mux(0x4 + 4 * 5, mux(FUN_WPU | FUN_WPD));
        assert_eq!(gpio.read(0x3c).unwrap() & 1 << 5, 1 << 5, "kept high");
        gpio.write_io_mux(0x4 + 4 * 5, mux(FUN_WPD));
        assert_eq!(gpio.read(0x3c).unwrap() & 1 << 5, 0);
        gpio.write_io_mux(0x4 + 4 * 5, mux(FUN_WPU | FUN_WPD));
        assert_eq!(gpio.read(0x3c).unwrap() & 1 << 5, 0, "kept low");
    }

    fn pin4(gpio: &mut Gpio, trigger: u32, enabled: bool) {
        let value = trigger << INT_TYPE_SHIFT | u32::from(enabled) << INT_ENA_SHIFT;
        gpio.write(0x74 + 4 * 4, value).unwrap();
    }

    fn gpio4() -> Gpio {
        let mut gpio = Gpio::new(&[4]);
        gpio.write_io_mux(0x4 + 4 * 4, GPIO_FUNC | FUN_IE);
        gpio
    }

    #[test]
    fn open_drain_drives_off_or_pull_up_for_a_high_output() {
        let mut gpio = gpio21();
        gpio.write(0x24, 1 << 21).unwrap();
        gpio.write(0x08, 1 << 21).unwrap();
        gpio.write(0x74 + 4 * 21, PAD_DRIVER).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::Off);
        gpio.write_io_mux(0x4 + 4 * 21, GPIO_FUNC | FUN_IE | FUN_WPU);
        assert_eq!(gpio.drive(21), PadDrive::PullUp);
        gpio.write(0x0c, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::Low, "a low output still drives");
        gpio.write(0x74 + 4 * 21, 0).unwrap();
        gpio.write(0x08, 1 << 21).unwrap();
        assert_eq!(gpio.drive(21), PadDrive::High, "push-pull");
        assert_eq!(gpio.read(0x74 + 4 * 21), Some(0));

        gpio.write_io_mux(0x4 + 4 * 5, GPIO_FUNC | FUN_IE);
        gpio.write(0x24, 1 << 5).unwrap();
        gpio.write(0x08, 1 << 5).unwrap();
        gpio.write(0x74 + 4 * 5, PAD_DRIVER).unwrap();
        assert_eq!(
            gpio.read(0x3c).unwrap() & 1 << 5,
            0,
            "a pad with no net floats low"
        );
    }

    #[test]
    fn the_input_matrix_reads_the_pad_in_sel_names_inverted_or_not() {
        const SIG_IN_SEL: u32 = 1 << 7;
        let mut gpio = gpio21();
        let scl = 0x154 + 4 * 89;
        assert_eq!(gpio.input(89), None, "on IO_MUX at reset");
        gpio.write(scl, SIG_IN_SEL | 21).unwrap();
        assert_eq!(gpio.input(89), Some(false));
        gpio.set_pad(21, true);
        assert_eq!(gpio.input(89), Some(true));
        gpio.write(scl, SIG_IN_SEL | 1 << 6 | 21).unwrap();
        assert_eq!(gpio.input(89), Some(false), "inverted");
        gpio.write(scl, SIG_IN_SEL | 0x38).unwrap();
        assert_eq!(gpio.input(89), Some(true), "constant 1");
        gpio.write(scl, SIG_IN_SEL | 0x3c).unwrap();
        assert_eq!(gpio.input(89), Some(false), "constant 0");
        gpio.write(scl, SIG_IN_SEL | 0x31).unwrap();
        assert_eq!(gpio.input(89), None, "no pad 49");
        assert_eq!(gpio.input(90), None, "each signal its own word");
    }

    #[test]
    fn an_edge_on_a_disabled_pin_is_latched_but_not_pending() {
        let mut gpio = gpio4();
        gpio.set_pad(4, true);
        pin4(&mut gpio, 2, false);
        gpio.set_pad(4, false);
        assert_eq!(gpio.read(0x44), Some(1 << 4));
        assert_eq!(gpio.read(0x5c), Some(0));
        assert_eq!(gpio.pending(), 0);
        gpio.write(0x74 + 4 * 5, 2 << INT_TYPE_SHIFT | 1 << INT_ENA_SHIFT)
            .unwrap();
        assert_eq!(gpio.read(0x5c), Some(0), "another pin enabled");
        pin4(&mut gpio, 2, true);
        assert_eq!(gpio.read(0x5c), Some(1 << 4), "pending once enabled");
        gpio.write(0x4c, 1 << 4).unwrap();
        assert_eq!(gpio.read(0x44), Some(0));
        assert_eq!(gpio.pending(), 0);
        gpio.set_pad(4, true);
        assert_eq!(gpio.pending(), 0, "a rise is not a fall");
    }

    #[test]
    fn each_edge_type_latches_its_edges() {
        for (trigger, rise, fall) in [
            (0, false, false),
            (1, true, false),
            (2, false, true),
            (3, true, true),
        ] {
            let mut gpio = gpio4();
            pin4(&mut gpio, trigger, true);
            gpio.set_pad(4, true);
            assert_eq!(gpio.pending() != 0, rise, "type {trigger}, rise");
            gpio.write(0x4c, 1 << 4).unwrap();
            gpio.set_pad(4, false);
            assert_eq!(gpio.pending() != 0, fall, "type {trigger}, fall");
        }
    }

    #[test]
    fn a_level_low_pin_reads_pending_again_after_w1tc_while_low() {
        let mut gpio = gpio4();
        gpio.set_pad(4, true);
        pin4(&mut gpio, 4, true);
        assert_eq!(gpio.pending(), 0);
        gpio.set_pad(4, false);
        assert_eq!(gpio.pending(), 1 << 4);
        gpio.write(0x4c, 1 << 4).unwrap();
        assert_eq!(gpio.pending(), 1 << 4, "still low");
        assert_eq!(gpio.read(0x44), Some(1 << 4));
        gpio.set_pad(4, true);
        assert_eq!(gpio.pending(), 0, "nothing latched");
        pin4(&mut gpio, 5, true);
        assert_eq!(gpio.pending(), 1 << 4, "level high");
    }

    #[test]
    fn the_high_word_holds_gpio32_and_up() {
        let mut gpio = Gpio::new(&[40]);
        gpio.write_io_mux(0x4 + 4 * 40, GPIO_FUNC | FUN_IE);
        gpio.write(0x74 + 4 * 40, 1 << INT_TYPE_SHIFT | 1 << INT_ENA_SHIFT)
            .unwrap();
        gpio.set_pad(40, true);
        assert_eq!(gpio.read(0x50), Some(1 << 8));
        assert_eq!(gpio.read(0x68), Some(1 << 8));
        assert_eq!(gpio.read(0x5c), Some(0));
        gpio.write(0x58, 1 << 8).unwrap();
        assert_eq!(gpio.read(0x68), Some(0));
    }
}
