//! The peripheral blocks by base address, and which of them the model simulates.
//!
//! The bases are IDF's `soc/esp32s3/register/soc/reg_base.h` and, for the four tables after
//! `EXTMEM`, `soc/esp32s3/include/soc/soc.h`. `I2C_MST` is the analog I2C master at
//! `I2C_MST_ANA_CONF0_REG`'s block, and `USB_OTG` is TinyUSB's `DWC2_FS_REG_BASE`. `RADIO` is the
//! space between UART2 and USB Serial/JTAG, where Wi-Fi and Bluetooth keep the registers IDF
//! does not document, such as `WDEV_RND_REG`. `RESERVED` fills each gap the headers leave, so
//! every address in peripheral space has a row. SENS's touch registers,
//! `SENS_SAR_TOUCH_CONF_REG` to `SENS_SAR_TOUCH_APPR_STATUS_REG`, are a row of their own, named
//! SENS.
//!
//! A register that a model keeps, or that [`quiet`](super::soc) names, works whatever its row
//! says. An access to any other register in a row the model runs reads 0, drops its write and is
//! listed once as an unmodeled register. In a row it does not simulate, that access ends the run
//! and names the block.

/// What the model does with the registers no model keeps in a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sim {
    /// The model runs the block, or keeps its registers from the boot.
    Runs,
    /// The model does not simulate the block, which these words name for a maker.
    Not(&'static str),
}

use Sim::{Not, Runs};

const RESERVED: Sim = Not("an address with no documented peripheral");
const RADIO: Sim = Not("the Wi-Fi and Bluetooth radio");
const SDIO: Sim = Not("the SDIO slave");
const UHCI: Sim = Not("a UART DMA controller");
const I2S: Sim = Not("an I2S audio port");
const MCPWM: Sim = Not("the motor-control PWM that ESP32Servo drives servos with");
const OTG: Sim = Not("the USB OTG port that TinyUSB, USB MSC and USB HID use");
const CACHE_TAGS: Sim = Not("the cache tag memory");

const BLOCKS: &[(u32, &str, Sim)] = &[
    (0x6000_0000, "UART0", Runs),
    (0x6000_1000, "RESERVED", RESERVED),
    (0x6000_2000, "SPI1", Runs),
    (0x6000_3000, "SPI0", Runs),
    (0x6000_4000, "GPIO", Runs),
    (
        0x6000_4f00,
        "GPIO_SD",
        Not("the sigma-delta output that sigmaDeltaWrite uses"),
    ),
    (0x6000_5000, "FE2", RADIO),
    (0x6000_6000, "FE", RADIO),
    (0x6000_7000, "EFUSE", Runs),
    (0x6000_8000, "RTC_CNTL", Runs),
    (0x6000_8400, "RTCIO", Runs),
    (0x6000_8800, "SENS", Runs),
    (
        0x6000_885c,
        "SENS",
        Not("the touch sensor that touchRead uses"),
    ),
    (0x6000_88e4, "SENS", Runs),
    (
        0x6000_8c00,
        "RTC_I2C",
        Not("the RTC I2C controller of the ULP coprocessor"),
    ),
    (0x6000_9000, "IO_MUX", Runs),
    (0x6000_a000, "RESERVED", RESERVED),
    (0x6000_b000, "HINF", SDIO),
    (0x6000_c000, "UHCI1", UHCI),
    (0x6000_d000, "RESERVED", RESERVED),
    (0x6000_e000, "I2C_MST", Runs),
    (0x6000_f000, "I2S0", I2S),
    (0x6001_0000, "UART1", Runs),
    (0x6001_1000, "BT", Not("the Bluetooth radio")),
    (0x6001_2000, "RESERVED", RESERVED),
    (0x6001_3000, "I2C0", Runs),
    (0x6001_4000, "UHCI0", UHCI),
    (0x6001_5000, "SLCHOST", SDIO),
    (0x6001_6000, "RMT", Runs),
    (0x6001_7000, "PCNT", Runs),
    (0x6001_8000, "SLC", SDIO),
    (0x6001_9000, "LEDC", Runs),
    (0x6001_a000, "RESERVED", RESERVED),
    (0x6001_cc00, "NRX", RADIO),
    (0x6001_d000, "BB", RADIO),
    (0x6001_e000, "MCPWM0", MCPWM),
    (0x6001_f000, "TIMG0", Runs),
    (0x6002_0000, "TIMG1", Runs),
    (
        0x6002_1000,
        "RTC_SLOWMEM",
        Not("RTC slow memory on the peripheral bus"),
    ),
    (0x6002_2000, "RESERVED", RESERVED),
    (0x6002_3000, "SYSTIMER", Runs),
    (0x6002_4000, "SPI2", Runs),
    (
        0x6002_5000,
        "SPI3",
        Not("the second SPI port, SPIClass(HSPI)"),
    ),
    (0x6002_6000, "SYSCON", Runs),
    (0x6002_7000, "I2C1", Not("the second I2C port, Wire1")),
    (
        0x6002_8000,
        "SDMMC",
        Not("the SD card host that SD_MMC uses"),
    ),
    (0x6002_9000, "RESERVED", RESERVED),
    (
        0x6002_a000,
        "PERI_BACKUP",
        Not("the peripheral backup that light sleep uses"),
    ),
    (0x6002_b000, "TWAI", Not("the CAN bus controller")),
    (0x6002_c000, "MCPWM1", MCPWM),
    (0x6002_d000, "I2S1", I2S),
    (0x6002_e000, "UART2", Runs),
    (0x6002_f000, "RADIO", RADIO),
    (0x6003_8000, "USB_SERIAL_JTAG", Runs),
    (0x6003_9000, "USB_WRAP", OTG),
    (0x6003_a000, "AES", Not("the AES accelerator")),
    (0x6003_b000, "SHA", Runs),
    (0x6003_c000, "RSA", Not("the RSA accelerator")),
    (
        0x6003_d000,
        "DIGITAL_SIGNATURE",
        Not("the digital signature peripheral"),
    ),
    (0x6003_e000, "HMAC", Not("the HMAC accelerator")),
    (0x6003_f000, "GDMA", Runs),
    (0x6004_0000, "APB_SARADC", Runs),
    (0x6004_1000, "LCD_CAM", Not("the LCD and camera controller")),
    (0x6004_2000, "RESERVED", RESERVED),
    (0x6008_0000, "USB_OTG", OTG),
    (0x600c_0000, "SYSTEM", Runs),
    (0x600c_1000, "SENSITIVE", Runs),
    (0x600c_2000, "INTERRUPT", Runs),
    (0x600c_3000, "RESERVED", RESERVED),
    (0x600c_4000, "EXTMEM", Runs),
    (0x600c_5000, "MMU_TABLE", Runs),
    (0x600c_6000, "ITAG_TABLE", CACHE_TAGS),
    (0x600c_8000, "DTAG_TABLE", CACHE_TAGS),
    (
        0x600c_c000,
        "EXT_MEM_ENC",
        Not("the flash and PSRAM encryption"),
    ),
    (0x600c_e000, "ASSIST_DEBUG", Not("the debug assist unit")),
    (0x600c_f000, "RESERVED", RESERVED),
    (0x600d_0000, "WCL", Runs),
];

/// The row at or below `addr`, and the base of its block: the first of the rows before it that
/// share its name.
fn row(addr: u32) -> Option<(u32, &'static str, Sim)> {
    let k = BLOCKS
        .partition_point(|&(base, ..)| base <= addr)
        .checked_sub(1)?;
    let (_, block, sim) = BLOCKS[k];
    let first = BLOCKS[..=k]
        .iter()
        .rev()
        .take_while(|&&(_, name, _)| name == block)
        .last()
        .expect("the row itself");
    Some((first.0, block, sim))
}

/// `BLOCK+0x1c` for the block at or below `addr`, `PERIPHERAL+0x...` below every block.
pub(crate) fn name(addr: u32) -> String {
    match row(addr) {
        Some((base, block, _)) => format!("{block}+{:#x}", addr - base),
        None => format!("PERIPHERAL+{addr:#x}"),
    }
}

/// The block holding `addr` and what it is, when the model does not simulate it.
pub(crate) fn unsimulated(addr: u32) -> Option<(&'static str, &'static str)> {
    match row(addr)? {
        (_, block, Not(what)) => Some((block, what)),
        (.., Runs) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_sorted_and_name_an_address_by_its_block() {
        assert!(BLOCKS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(name(0x6000_401c), "GPIO+0x1c");
        assert_eq!(name(0x6000_4f04), "GPIO_SD+0x4");
        assert_eq!(name(0x600c_0018), "SYSTEM+0x18");
        assert_eq!(name(0x600c_5004), "MMU_TABLE+0x4");
        assert_eq!(name(0x600c_cffc), "EXT_MEM_ENC+0xffc");
        assert_eq!(name(0x6000_8850), "SENS+0x50");
        assert_eq!(name(0x6000_889c), "SENS+0x9c");
        assert_eq!(name(0x6000_88e4), "SENS+0xe4");
        assert_eq!(name(0x6003_1204), "RADIO+0x2204");
    }

    /// The blocks a stock sketch's boot and the modelled peripherals use run. Every other block
    /// is refused by name, with words a maker knows.
    #[test]
    fn every_block_the_model_does_not_run_is_refused_by_name() {
        let runs = [
            "UART0",
            "SPI1",
            "SPI0",
            "GPIO",
            "EFUSE",
            "RTC_CNTL",
            "RTCIO",
            "SENS",
            "IO_MUX",
            "I2C_MST",
            "UART1",
            "I2C0",
            "RMT",
            "PCNT",
            "LEDC",
            "TIMG0",
            "TIMG1",
            "SYSTIMER",
            "SPI2",
            "SYSCON",
            "UART2",
            "USB_SERIAL_JTAG",
            "SHA",
            "GDMA",
            "APB_SARADC",
            "SYSTEM",
            "SENSITIVE",
            "INTERRUPT",
            "EXTMEM",
            "MMU_TABLE",
            "WCL",
        ];
        for &(base, block, sim) in BLOCKS {
            if block == "SENS" {
                continue;
            }
            match sim {
                Runs => assert!(runs.contains(&block), "{block} runs"),
                Not(what) => {
                    assert!(!runs.contains(&block), "{block}");
                    assert_eq!(unsimulated(base), Some((block, what)));
                    assert_eq!(unsimulated(base + 4), Some((block, what)));
                }
            }
        }
        let refused = [
            (
                0x6001_e10c,
                "MCPWM0",
                "the motor-control PWM that ESP32Servo drives servos with",
            ),
            (0x6000_f034, "I2S0", "an I2S audio port"),
            (0x6002_501c, "SPI3", "the second SPI port, SPIClass(HSPI)"),
            (0x6002_b000, "TWAI", "the CAN bus controller"),
            (0x6002_7054, "I2C1", "the second I2C port, Wire1"),
            (0x6003_1204, "RADIO", "the Wi-Fi and Bluetooth radio"),
            (0x6001_1000, "BT", "the Bluetooth radio"),
            (0x6000_889c, "SENS", "the touch sensor that touchRead uses"),
            (0x6000_885c, "SENS", "the touch sensor that touchRead uses"),
            (0x6000_88e0, "SENS", "the touch sensor that touchRead uses"),
            (
                0x6003_9000,
                "USB_WRAP",
                "the USB OTG port that TinyUSB, USB MSC and USB HID use",
            ),
            (
                0x6008_0000,
                "USB_OTG",
                "the USB OTG port that TinyUSB, USB MSC and USB HID use",
            ),
            (
                0x6004_5000,
                "RESERVED",
                "an address with no documented peripheral",
            ),
        ];
        for (addr, block, what) in refused {
            assert_eq!(unsimulated(addr), Some((block, what)), "{addr:#x}");
        }
        for addr in [
            0x6000_884c,
            0x6000_8850,
            0x6000_8854,
            0x6000_8858,
            0x6000_88e4,
            0x6000_8488,
            0x6000_e040,
            0x6001_7030,
            0x600d_0000,
        ] {
            assert_eq!(unsimulated(addr), None, "{addr:#x}");
        }
        assert_eq!(unsimulated(0x5fff_fffc), None);
    }
}
