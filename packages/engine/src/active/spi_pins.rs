//! An SPI target on the pins, for a controller that clocks SPI on its pads, as the ATmega328P's
//! SPI does. It watches CS, SCLK and MOSI and drives MISO, in mode 0, most significant bit
//! first, and calls the device through [`SpiTarget`] as the record engine does. No part hosts it
//! yet. The ATmega328P read-back test does.

use std::sync::Arc;

use orbe_model::{Cs, Drive, Lanes, Miso, Spi, SpiTarget, Time};

use super::native::{Hosting, Port};
use crate::world::World;

/// 10 ns from an SCLK fall, or from CS falling, to the next MISO bit, this model's choice. It is
/// under the 25 ns half period of the fastest SCLK an ATmega328P makes, at 20 MHz over 2.
pub(crate) const MISO_DELAY_PS: u64 = 10_000;

/// Which of a device's watches are CS, active low, SCLK and MOSI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lines {
    pub(crate) cs: usize,
    pub(crate) sclk: usize,
    pub(crate) mosi: usize,
}

/// A target on the pins. CS falling starts a transaction and asks for the first byte, each SCLK
/// rise samples MOSI, and each SCLK fall shifts out the next bit, asking for the next byte after
/// a whole one came in. CS rising ends it: the device gets the whole bytes as one full-duplex
/// [`Spi`], and MISO is released. A byte the device does not answer leaves MISO released.
#[derive(Clone, Debug)]
pub(crate) struct Pins {
    lines: Lines,
    /// The levels at the last input, from power-on.
    cs: bool,
    sclk: bool,
    selected: bool,
    /// The bits of the byte coming in, and how many.
    shifter: u8,
    bits: u8,
    /// The byte going out, or `None` while MISO is released for it.
    out: Option<u8>,
    data: Vec<u8>,
    /// The first two SCLK rises of the transaction, for its clock.
    rises: [Option<Time>; 2],
    miso: Drive,
    due: Option<(Time, Drive)>,
}

impl Pins {
    pub(crate) const fn new(lines: Lines) -> Pins {
        Pins {
            lines,
            cs: true,
            sclk: false,
            selected: false,
            shifter: 0,
            bits: 0,
            out: None,
            data: Vec::new(),
            rises: [None; 2],
            miso: Drive::Off,
            due: None,
        }
    }

    /// Bit `bit` of the byte going out, from `at` plus the delay.
    fn shift(&mut self, at: Time, bit: u8) {
        let drive = match self.out {
            Some(byte) if byte >> bit & 1 == 1 => Drive::High,
            Some(_) => Drive::Low,
            None => Drive::Off,
        };
        self.due = Some((Time::from_ps(at.as_ps() + MISO_DELAY_PS), drive));
    }

    /// Asks for the byte that goes out beside data byte `data.len()`, and puts out its bit 7.
    fn next_byte<D: SpiTarget>(&mut self, dev: &mut Hosting<D>, world: &World, at: Time) {
        let (data, index) = (&self.data, self.data.len());
        self.out = dev.call(world, at, |device, cx| {
            device.answer(cx, at, &[], data, index)
        });
        self.shift(at, 7);
    }

    fn end<D: SpiTarget>(&mut self, dev: &mut Hosting<D>, world: &World, at: Time) {
        self.selected = false;
        self.due = Some((Time::from_ps(at.as_ps() + MISO_DELAY_PS), Drive::Off));
        let data = std::mem::take(&mut self.data);
        if data.is_empty() {
            return;
        }
        let clock_hz = match self.rises {
            [Some(first), Some(second)] => {
                let period = (second.as_ps() - first.as_ps()).max(1);
                u32::try_from(1_000_000_000_000 / period).unwrap_or(u32::MAX)
            }
            _ => 0,
        };
        let spi = Spi {
            phases: Vec::new(),
            lanes: Lanes::One,
            data: Arc::from(data),
            clock_hz,
            cs: Cs::Released,
            miso: Miso::Duplex,
        };
        dev.call(world, at, |device, cx| device.transaction(cx, at, &spi));
    }
}

impl<D: SpiTarget> Port<D> for Pins {
    const PINS: usize = 1;

    fn drive(&self, image: &mut [Drive]) {
        image[0] = self.miso;
    }

    fn due_at(&self) -> Option<Time> {
        self.due.map(|(at, _)| at)
    }

    fn take_due(&mut self, _dev: &mut Hosting<D>, _world: &World, at: Time) -> bool {
        let (due, drive) = self.due.take().expect("a drive is due");
        debug_assert_eq!(due, at);
        self.miso = drive;
        false
    }

    fn input(&mut self, dev: &mut Hosting<D>, world: &World, at: Time, levels: &[bool]) {
        let (cs, sclk) = (levels[self.lines.cs], levels[self.lines.sclk]);
        let (cs_fell, cs_rose) = (!cs && self.cs, cs && !self.cs);
        let (rose, fell) = (sclk && !self.sclk, !sclk && self.sclk);
        self.cs = cs;
        self.sclk = sclk;
        if cs_rose && self.selected {
            self.end(dev, world, at);
        } else if cs_fell {
            self.selected = true;
            self.bits = 0;
            self.rises = [None; 2];
            self.next_byte(dev, world, at);
        } else if self.selected && rose {
            self.shifter = self.shifter << 1 | u8::from(levels[self.lines.mosi]);
            self.bits += 1;
            if let Some(slot) = self.rises.iter_mut().find(|rise| rise.is_none()) {
                *slot = Some(at);
            }
            if self.bits == 8 {
                self.data.push(self.shifter);
                self.bits = 0;
            }
        } else if self.selected && fell {
            if self.bits == 0 {
                self.next_byte(dev, world, at);
            } else {
                self.shift(at, 7 - self.bits);
            }
        }
    }
}
