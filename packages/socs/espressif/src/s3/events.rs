//! The next event of each peripheral a poll asks, kept from one poll to the next.
//!
//! A peripheral's next event moves at that event, at an access to its registers, or when a pad
//! it samples changes. Inside a slice that runs the cores a pad changes only at an access, since
//! the run drives the UART's edges and takes the host's input between slices. So the chip forgets
//! every event when such a slice starts, an access from either core forgets the events it can
//! move, and a poll asks again only the peripherals whose event it forgot or has reached. Each
//! answer equals the one asking every peripheral gives, which a build with debug assertions
//! checks at each poll.

use super::clock::Tick;
use super::soc::{LEDC, RMT, SPI2, SYSTIMER, TIMG0, TIMG1, USB_SERIAL_JTAG};
use super::uart::Uarts;
use super::usb;

/// The peripherals a poll asks, each a slot of [`Events`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Peripheral {
    Systimer,
    /// The general timers of both timer groups.
    Timers,
    Spi2,
    Usb,
    Rmt,
    Uart,
    Ledc,
}

impl Peripheral {
    pub(crate) const ALL: [Peripheral; 7] = [
        Peripheral::Systimer,
        Peripheral::Timers,
        Peripheral::Spi2,
        Peripheral::Usb,
        Peripheral::Rmt,
        Peripheral::Uart,
        Peripheral::Ledc,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

const EVERY: u8 = (1 << Peripheral::ALL.len()) - 1;

#[derive(Clone, Debug, Default)]
pub(crate) struct Events {
    at: [Option<Tick>; Peripheral::ALL.len()],
    /// Bit `n` is set while slot `n` holds the answer.
    held: u8,
}

impl Events {
    /// The event `peripheral` gave at the last poll that asked it, unless an access forgot it or
    /// `now` has reached it.
    #[inline]
    pub(crate) fn kept(&self, peripheral: Peripheral, now: Tick) -> Option<Option<Tick>> {
        let at = self.at[peripheral as usize];
        (self.held & peripheral.bit() != 0 && at.is_none_or(|at| at > now)).then_some(at)
    }

    pub(crate) fn keep(&mut self, peripheral: Peripheral, at: Option<Tick>) {
        self.at[peripheral as usize] = at;
        self.held |= peripheral.bit();
    }

    pub(crate) fn forget(&mut self) {
        self.held = 0;
    }

    /// Forgets the events an access to the register at `addr` can move. The UART's receivers
    /// sample their pads, so an access that can drive a pad forgets the UART's event too. An
    /// access to a block not named here, such as the GPIO matrix, the clocks or the gates,
    /// forgets every event.
    #[inline]
    pub(crate) fn touch(&mut self, addr: u32) {
        let moved = match addr & !0xfff {
            SYSTIMER => Peripheral::Systimer.bit(),
            TIMG0 | TIMG1 => Peripheral::Timers.bit(),
            USB_SERIAL_JTAG if addr & !3 == USB_SERIAL_JTAG + usb::CONF0 => {
                Peripheral::Usb.bit() | Peripheral::Uart.bit()
            }
            USB_SERIAL_JTAG => Peripheral::Usb.bit(),
            RMT => Peripheral::Rmt.bit() | Peripheral::Uart.bit(),
            SPI2 => Peripheral::Spi2.bit() | Peripheral::Uart.bit(),
            LEDC => Peripheral::Ledc.bit(),
            base if Uarts::index(base).is_some() => Peripheral::Uart.bit(),
            _ => EVERY,
        };
        self.held &= !moved;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::soc::GPIO;

    #[test]
    fn a_kept_event_holds_until_its_tick() {
        let mut events = Events::default();
        assert_eq!(
            events.kept(Peripheral::Uart, Tick(0)),
            None,
            "nothing kept yet"
        );
        events.keep(Peripheral::Uart, Some(Tick(10)));
        assert_eq!(events.kept(Peripheral::Uart, Tick(9)), Some(Some(Tick(10))));
        assert_eq!(events.kept(Peripheral::Uart, Tick(10)), None, "reached");
        events.keep(Peripheral::Rmt, None);
        assert_eq!(events.kept(Peripheral::Rmt, Tick(u64::MAX)), Some(None));
    }

    #[test]
    fn an_access_forgets_the_events_it_can_move() {
        let mut events = Events::default();
        let keep_all = |events: &mut Events| {
            for peripheral in Peripheral::ALL {
                events.keep(peripheral, None);
            }
        };
        let left = |events: &Events| {
            Peripheral::ALL.map(|peripheral| events.kept(peripheral, Tick(0)).is_some())
        };
        keep_all(&mut events);
        events.touch(SYSTIMER + 0x64);
        assert_eq!(left(&events), [false, true, true, true, true, true, true]);
        keep_all(&mut events);
        events.touch(TIMG1 + 0x10);
        assert_eq!(left(&events), [true, false, true, true, true, true, true]);
        keep_all(&mut events);
        events.touch(USB_SERIAL_JTAG);
        assert_eq!(left(&events), [true, true, true, false, true, true, true]);
        keep_all(&mut events);
        events.touch(RMT + 0x20);
        assert_eq!(left(&events), [true, true, true, true, false, false, true]);
        keep_all(&mut events);
        events.touch(SPI2);
        assert_eq!(left(&events), [true, true, false, true, true, false, true]);
        keep_all(&mut events);
        events.touch(crate::s3::uart::BASES[2] + 0x1c);
        assert_eq!(left(&events), [true, true, true, true, true, false, true]);
        keep_all(&mut events);
        events.touch(LEDC + 0xcc);
        assert_eq!(left(&events), [true, true, true, true, true, true, false]);
        keep_all(&mut events);
        events.touch(GPIO + 4);
        assert_eq!(left(&events), [false; 7]);
        keep_all(&mut events);
        events.forget();
        assert_eq!(left(&events), [false; 7]);
    }
}
