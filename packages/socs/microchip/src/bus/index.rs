//! Device indexes resolved once, when the chip is built.
//!
//! An I/O access names a slot in the owner table. A device the bus needs on
//! an access is found through these indexes, not by walking `devices`.

use crate::chip::DeviceSpec;

/// One slot per kind of device.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Index {
    pub adc: Option<u8>,
    pub acmp: Option<u8>,
    pub system: Option<u8>,
    pub eeprom: Option<u8>,
    pub spi: Option<u8>,
    pub twi: Option<u8>,
    pub usart: [Option<u8>; 1],
    pub prescaler: Option<u8>,
    pub extint: Option<u8>,
    /// Timer whose capture input follows `ACO` when `ACIC` is set.
    pub acic: Option<u8>,
    pub timers: [Option<u8>; 3],
    pub timer_n: u8,
    /// Devices that queue a [`crate::periph::Notice`]. Built with the indexes.
    pub notices: [Option<u8>; 8],
    pub notice_n: u8,
}

impl Index {
    pub(crate) const fn empty() -> Self {
        Self {
            adc: None,
            acmp: None,
            system: None,
            eeprom: None,
            spi: None,
            twi: None,
            usart: [None; 1],
            prescaler: None,
            extint: None,
            acic: None,
            timers: [None; 3],
            timer_n: 0,
            notices: [None; 8],
            notice_n: 0,
        }
    }

    /// `specs` is the chip's device list, in the order the bus stores them.
    pub(crate) fn from_specs(specs: &[DeviceSpec]) -> Self {
        let mut index = Self::empty();
        let mut usart_n = 0usize;
        for (slot, spec) in specs.iter().enumerate() {
            let id = u8::try_from(slot).expect("device index fits in a byte");
            match spec {
                DeviceSpec::Adc(_) => {
                    index.adc = Some(id);
                    index.push_notice(id);
                }
                DeviceSpec::Acmp(_) => {
                    index.acmp = Some(id);
                    index.push_notice(id);
                }
                DeviceSpec::System(_) => index.system = Some(id),
                DeviceSpec::Eeprom(_) => index.eeprom = Some(id),
                DeviceSpec::Spi(_) => index.spi = Some(id),
                DeviceSpec::Twi(_) => index.twi = Some(id),
                DeviceSpec::Usart(_) => {
                    if usart_n < index.usart.len() {
                        index.usart[usart_n] = Some(id);
                        usart_n += 1;
                    }
                }
                DeviceSpec::Prescaler(_) => index.prescaler = Some(id),
                DeviceSpec::ExtInt(_) => {
                    index.extint = Some(id);
                    index.push_notice(id);
                }
                DeviceSpec::Timer(cfg) => {
                    if usize::from(index.timer_n) < index.timers.len() {
                        index.timers[usize::from(index.timer_n)] = Some(id);
                        index.timer_n += 1;
                    }
                    if cfg.acic {
                        index.acic = Some(id);
                    }
                    index.push_notice(id);
                }
                DeviceSpec::Port(_) => {}
            }
        }
        index
    }

    fn push_notice(&mut self, id: u8) {
        if usize::from(self.notice_n) < self.notices.len() {
            self.notices[usize::from(self.notice_n)] = Some(id);
            self.notice_n += 1;
        }
    }
}
