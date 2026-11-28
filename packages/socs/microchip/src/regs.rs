//! Register ids and the per-register strobe masks carried in the chip table.
//!
//! Each device matches on `id`. The bus never re-decodes an address. `strobe`
//! is the `SBI`/`CBI` mask (write-one-to-clear flags, or every bit of `PINx`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OwnedReg {
    pub addr: u16,
    pub id: u8,
    pub strobe: u8,
}

pub(crate) const fn owned(addr: u16, id: u8, strobe: u8) -> OwnedReg {
    OwnedReg { addr, id, strobe }
}

/// `PINx`, `DDRx`, `PORTx`.
pub(crate) const GPIO_PIN: u8 = 0;
pub(crate) const GPIO_DDR: u8 = 1;
pub(crate) const GPIO_PORT: u8 = 2;

/// `GTCCR`.
pub(crate) const GTCCR: u8 = 0;

pub(crate) const TCCR_A: u8 = 0;
pub(crate) const TCCR_B: u8 = 1;
pub(crate) const TCCR_C: u8 = 2;
pub(crate) const TCNT_L: u8 = 3;
pub(crate) const TCNT_H: u8 = 4;
pub(crate) const ICR_L: u8 = 5;
pub(crate) const ICR_H: u8 = 6;
pub(crate) const OCR_A: u8 = 7;
pub(crate) const OCR_AH: u8 = 8;
pub(crate) const OCR_B: u8 = 9;
pub(crate) const OCR_BH: u8 = 10;
pub(crate) const TIMSK: u8 = 11;
pub(crate) const TIFR: u8 = 12;
pub(crate) const ASSR: u8 = 13;

pub(crate) const UCSRA: u8 = 0;
pub(crate) const UCSRB: u8 = 1;
pub(crate) const UCSRC: u8 = 2;
pub(crate) const UBRRL: u8 = 3;
pub(crate) const UBRRH: u8 = 4;
pub(crate) const UDR: u8 = 5;

/// `UCSRnA.TXC` is write-one-to-clear (DS40002061B §20.11.2).
pub(crate) const UCSRA_TXC_STROBE: u8 = 1 << 6;

/// SPI register ids. `SPSR` bits 5:1 are reserved (§19.5.2).
pub(crate) const SPCR: u8 = 0;
pub(crate) const SPSR: u8 = 1;
pub(crate) const SPDR: u8 = 2;

/// TWI register ids. `TWCR` bit 1 and `TWSR` bit 2 read 0. `TWAMR` bit 0 reads 0.
pub(crate) const TWBR: u8 = 0;
pub(crate) const TWSR: u8 = 1;
pub(crate) const TWAR: u8 = 2;
pub(crate) const TWDR: u8 = 3;
pub(crate) const TWCR: u8 = 4;
pub(crate) const TWAMR: u8 = 5;

pub(crate) const REG_CAP: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegBlock {
    pub rows: [OwnedReg; REG_CAP],
    pub len: u8,
}

pub(crate) const fn reg_block() -> RegBlock {
    RegBlock {
        rows: [OwnedReg {
            addr: 0,
            id: 0,
            strobe: 0,
        }; REG_CAP],
        len: 0,
    }
}

pub(crate) const fn push_reg(block: &mut RegBlock, row: OwnedReg) {
    block.rows[block.len as usize] = row;
    block.len += 1;
}
