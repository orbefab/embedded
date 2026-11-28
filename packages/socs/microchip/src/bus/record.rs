//! Memory snapshots, enable windows, and the device slots.
//!
//! Sleep and the fuse clocks stay in [`super::power`]. These methods are the
//! side record a host reads after a run, and the indexes the bus uses to
//! reach a device without scanning the list.

use super::Bus;
use crate::periph::Device;

impl Bus {
    pub(crate) fn eeprom_bytes(&self) -> Vec<u8> {
        self.eeprom()
            .map(|ee| ee.bytes().to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn set_eeprom(&mut self, bytes: &[u8]) {
        if let Some(ee) = self.eeprom_mut() {
            ee.load_image(bytes);
        }
    }

    /// §8.4.2. Store the byte a reset is about to interrupt, when `VCC` holds.
    pub(crate) fn commit_eeprom_pending(&mut self) {
        if let Some(ee) = self.eeprom_mut() {
            ee.commit_pending();
        }
    }

    pub(crate) fn sram_snapshot(&self) -> Vec<u8> {
        self.sram.clone()
    }

    pub(crate) fn restore_sram(&mut self, bytes: &[u8]) {
        let n = self.sram.len().min(bytes.len());
        self.sram[..n].copy_from_slice(&bytes[..n]);
    }

    pub(super) fn slot_dev(&self, slot: Option<u8>) -> Option<&Device> {
        self.devices.get(usize::from(slot?))
    }

    pub(super) fn slot_dev_mut(&mut self, slot: Option<u8>) -> Option<&mut Device> {
        self.devices.get_mut(usize::from(slot?))
    }

    pub(super) fn adc(&self) -> Option<&crate::periph::Adc> {
        match self.slot_dev(self.index.adc)? {
            Device::Adc(adc) => Some(adc),
            _ => None,
        }
    }

    pub(super) fn adc_mut(&mut self) -> Option<&mut crate::periph::Adc> {
        match self.slot_dev_mut(self.index.adc)? {
            Device::Adc(adc) => Some(adc),
            _ => None,
        }
    }

    pub(super) fn system(&self) -> Option<&crate::periph::system::System> {
        match self.slot_dev(self.index.system)? {
            Device::System(sys) => Some(sys),
            _ => None,
        }
    }

    pub(super) fn system_mut(&mut self) -> Option<&mut crate::periph::system::System> {
        match self.slot_dev_mut(self.index.system)? {
            Device::System(sys) => Some(sys),
            _ => None,
        }
    }

    pub(super) fn eeprom(&self) -> Option<&crate::periph::eeprom::Eeprom> {
        match self.slot_dev(self.index.eeprom)? {
            Device::Eeprom(ee) => Some(ee),
            _ => None,
        }
    }

    pub(super) fn eeprom_mut(&mut self) -> Option<&mut crate::periph::eeprom::Eeprom> {
        match self.slot_dev_mut(self.index.eeprom)? {
            Device::Eeprom(ee) => Some(ee),
            _ => None,
        }
    }

    #[inline(never)]
    pub(super) fn finish_windows(&mut self) {
        if !self.windows {
            return;
        }
        self.windows = false;
        let retire = self.cycle;
        for dev in &mut self.devices {
            dev.commit_windows(retire);
        }
        self.note_schedule();
    }
}
