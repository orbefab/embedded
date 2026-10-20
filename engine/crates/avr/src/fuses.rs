//! Fuse bytes and the delays the model reads out of them.
//!
//! DS40002061B §28.2 (Tables 28-6, 28-8, 28-9) and §9. A programmed bit is 0.
//! `cycles()` counts CPU clocks, so a start-up quoted in CK is that many CPU
//! cycles and a start-up quoted in milliseconds is `cpu_hz × time`.

/// Three fuse bytes. The host can replace them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuseBytes {
    pub low: u8,
    pub high: u8,
    pub extended: u8,
}

impl FuseBytes {
    /// Factory defaults, with `CKDIV8` left unprogrammed.
    ///
    /// The shipped low byte is `0x62` (`CKDIV8` programmed, internal 8 MHz RC,
    /// Table 9-12 `SUT = 10`). A bare [`crate::Cpu::new`] keeps that clock
    /// choice and start-up, but does not divide by 8, so `clock_hz` is the
    /// CPU clock until `set_fuses` says otherwise.
    /// High `0xD9` is `BOOTRST` unprogrammed and the default 2048-word boot
    /// section. Extended `0xFF` leaves `BODLEVEL` at 111, BOD disabled
    /// (Table 28-6, Table 29-12).
    pub const BARE: Self = Self {
        low: 0xE2,
        high: 0xD9,
        extended: 0xFF,
    };

    /// An external crystal with the boot section and brown-out set.
    ///
    /// Low `0xFF` is `CKDIV8` and `CKOUT` unprogrammed, the low-power crystal
    /// (`CKSEL3:1 = 111`, Table 9-3) with `CKSEL0 = 1` and `SUT = 11`, the
    /// slowest start-up of Table 9-4. High `0xDA` programs `SPIEN` and
    /// `BOOTRST` and sets `BOOTSZ = 01`, the 1024-word boot section (Table 28-8,
    /// Table 27-13). Extended `0xFD` is `BODLEVEL = 101`, typical `VBOT` 2.7 V
    /// (Table 28-6, Table 29-12).
    pub const CRYSTAL: Self = Self {
        low: 0xFF,
        high: 0xDA,
        extended: 0xFD,
    };
}

/// What the model needs from the three bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fuses {
    pub bytes: FuseBytes,
    /// `CKDIV8` programmed. Initial `CLKPS` is then 3 (divide by 8).
    pub ckdiv8: bool,
    /// `BOOTRST` programmed. The reset vector is the boot section.
    pub bootrst: bool,
    /// Word address of the boot section. Meaningful when `bootrst` is set.
    pub boot_word: u16,
    /// `WDTON` programmed. The watchdog stays in system-reset mode.
    pub wdton: bool,
    /// Typical `VBOT` in millivolts. `None` when BOD is disabled.
    pub bod_typ_mv: Option<u16>,
    pub startup: Startup,
}

/// CK counts and the extra reset delay. Milliseconds are microseconds here so
/// the arithmetic stays integral. 4.1 ms is 4100 µs. 65 ms is 65000 µs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Startup {
    /// Start-up from power-down and power-save, in CK.
    pub power_down_ck: u64,
    /// The CK part of the additional delay from reset.
    pub reset_ck: u64,
    /// The time part of the additional delay from reset, in microseconds.
    pub reset_us: u64,
}

impl Fuses {
    pub fn decode(bytes: FuseBytes) -> Self {
        let cksel = bytes.low & 0x0F;
        let sut = (bytes.low >> 4) & 0x03;
        let bootsz = (bytes.high >> 1) & 0x03;
        let bodlevel = bytes.extended & 0x07;
        Self {
            bytes,
            ckdiv8: bytes.low & (1 << 7) == 0,
            bootrst: bytes.high & 1 == 0,
            boot_word: boot_word(bootsz),
            wdton: bytes.high & (1 << 4) == 0,
            bod_typ_mv: bod_typical_mv(bodlevel),
            startup: startup_of(cksel, sut),
        }
    }

    /// Initial `CLKPS` code. Programmed `CKDIV8` is `0011` (§9.12.2).
    pub fn initial_clkps(self) -> u8 {
        if self.ckdiv8 { 0x03 } else { 0x00 }
    }

    pub fn clock_div(self) -> u32 {
        clkps_div(self.initial_clkps())
    }
}

/// Table 27-13. `BOOTSZ = 11` is 256 words at `0x3F00`, `01` is 1024 at `0x3C00`.
pub fn boot_word(bootsz: u8) -> u16 {
    match bootsz & 0x03 {
        0b11 => 0x3F00,
        0b10 => 0x3E00,
        0b01 => 0x3C00,
        _ => 0x3800,
    }
}

/// Table 29-12, typical column. 111 is disabled. Reserved codes stay disabled.
pub fn bod_typical_mv(level: u8) -> Option<u16> {
    match level & 0x07 {
        0b110 => Some(1800),
        0b101 => Some(2700),
        0b100 => Some(4300),
        _ => None,
    }
}

/// Table 29-11. `VHYST` typical is 50 mV. §11.5: `VBOT+ = VBOT + VHYST/2`.
pub const BOD_HYST_MV: u16 = 50;

pub fn bod_thresholds(typ_mv: u16) -> (u16, u16) {
    let half = BOD_HYST_MV / 2;
    (typ_mv.saturating_sub(half), typ_mv.saturating_add(half))
}

/// Falling threshold `VBOT-`, then rising `VBOT+`, in millivolts.
pub fn bod_window(level: u8) -> Option<(u16, u16)> {
    bod_typical_mv(level).map(bod_thresholds)
}

/// Table 9-17. Reserved codes divide by 1; the write that selects one is ignored
/// by the prescaler register, and this helper is only for a code that stuck.
pub fn clkps_div(clkps: u8) -> u32 {
    match clkps & 0x0F {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 32,
        6 => 64,
        7 => 128,
        8 => 256,
        _ => 1,
    }
}

pub fn clkps_reserved(clkps: u8) -> bool {
    clkps & 0x0F > 8
}

/// Which delay a wake or a reset uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayKind {
    /// Additional delay from reset (Table 9-4 and the matching tables).
    Reset,
    /// Power-down and power-save column.
    PowerDown,
    /// Standby and extended standby, §10.7 and §10.8. Six CK.
    Standby,
    /// Idle and ADC noise reduction. The main clock stayed on (Table 10-1).
    None,
}

impl Startup {
    pub fn cycles(self, kind: DelayKind, cpu_hz: u32) -> u64 {
        match kind {
            DelayKind::None => 0,
            DelayKind::Standby => 6,
            DelayKind::PowerDown => self.power_down_ck,
            DelayKind::Reset => {
                let timed = u64::from(cpu_hz).saturating_mul(self.reset_us) / 1_000_000;
                self.reset_ck.saturating_add(timed)
            }
        }
    }
}

/// Start-up rows from §9. `CKSEL` selects the table. `SUT` selects the row.
/// A reserved `SUT` uses the longest defined row of that table.
fn startup_of(cksel: u8, sut: u8) -> Startup {
    match cksel {
        // Tables 9-4 and 9-6. `CKSEL0` picks the row.
        0b0110..=0b1111 => crystal_row(cksel & 1, sut),
        // Table 9-12. The external clock (Table 9-16) has the same rows. The
        // 128 kHz and low-frequency crystal rows are not modelled and use it too.
        _ => rc_like(sut),
    }
}

/// Table 9-12 and Table 9-16. `SUT = 11` is reserved; use the slow row.
fn rc_like(sut: u8) -> Startup {
    let (ck, us) = match sut {
        0b00 => (14, 0),
        0b01 => (14, 4_100),
        _ => (14, 65_000),
    };
    Startup {
        power_down_ck: 6,
        reset_ck: ck,
        reset_us: us,
    }
}

/// Table 9-4 and Table 9-6.
fn crystal_row(cksel0: u8, sut: u8) -> Startup {
    let (pd, ck, us) = match (cksel0, sut) {
        (0, 0b00) => (258, 14, 4_100),
        (0, 0b01) => (258, 14, 65_000),
        (0, 0b10) => (1_024, 14, 0),
        (0, _) => (1_024, 14, 4_100),
        (_, 0b00) => (1_024, 14, 65_000),
        (_, 0b01) => (16_384, 14, 0),
        (_, 0b10) => (16_384, 14, 4_100),
        _ => (16_384, 14, 65_000),
    };
    Startup {
        power_down_ck: pd,
        reset_ck: ck,
        reset_us: us,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crystal_fuses_decode_to_the_crystal_row() {
        let f = Fuses::decode(FuseBytes::CRYSTAL);
        assert!(!f.ckdiv8);
        assert!(f.bootrst);
        assert_eq!(f.boot_word, 0x3C00);
        assert!(!f.wdton);
        assert_eq!(f.bod_typ_mv, Some(2700));
        assert_eq!(bod_window(0b101), Some((2675, 2725)));
        assert_eq!(f.initial_clkps(), 0);
        assert_eq!(f.clock_div(), 1);
        // Table 9-4, CKSEL0 = 1, SUT = 11: 16K CK, and 14 CK + 65 ms.
        assert_eq!(f.startup.power_down_ck, 16_384);
        assert_eq!(
            f.startup.cycles(DelayKind::Reset, 16_000_000),
            14 + 1_040_000
        );
        assert_eq!(f.startup.cycles(DelayKind::Standby, 16_000_000), 6);
        assert_eq!(f.startup.cycles(DelayKind::None, 16_000_000), 0);
    }

    #[test]
    fn bare_fuses_decode_to_the_internal_rc_row() {
        let f = Fuses::decode(FuseBytes::BARE);
        assert_eq!(f.bytes.low, 0xE2);
        // Table 9-12, SUT = 10: 6 CK from power-down, and 14 CK + 65 ms from reset.
        assert_eq!(f.startup.power_down_ck, 6);
        assert_eq!(f.startup.reset_ck, 14);
        assert_eq!(f.startup.reset_us, 65_000);
        assert_eq!(
            f.startup.cycles(DelayKind::Reset, 16_000_000),
            14 + 1_040_000
        );
    }

    #[test]
    fn external_clock_decodes_to_the_table_9_16_reset_row() {
        // Low `0xE0` is `CKSEL` 0000 and `SUT` 10.
        let f = Fuses::decode(FuseBytes {
            low: 0xE0,
            ..FuseBytes::BARE
        });
        assert_eq!(
            f.startup.cycles(DelayKind::Reset, 16_000_000),
            14 + 1_040_000
        );
    }
}
