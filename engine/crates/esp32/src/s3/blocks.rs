//! The peripheral blocks by base address, from IDF's `soc/esp32s3/register/soc/reg_base.h` and,
//! for the four tables after `EXTMEM`, `soc/esp32s3/include/soc/soc.h`, so the unmodeled-register
//! report can name an address by its block and offset.

const BLOCKS: &[(u32, &str)] = &[
    (0x6000_0000, "UART0"),
    (0x6000_2000, "SPI1"),
    (0x6000_3000, "SPI0"),
    (0x6000_4000, "GPIO"),
    (0x6000_4f00, "GPIO_SD"),
    (0x6000_5000, "FE2"),
    (0x6000_6000, "FE"),
    (0x6000_7000, "EFUSE"),
    (0x6000_8000, "RTC_CNTL"),
    (0x6000_8400, "RTCIO"),
    (0x6000_8800, "SENS"),
    (0x6000_8c00, "RTC_I2C"),
    (0x6000_9000, "IO_MUX"),
    (0x6000_b000, "HINF"),
    (0x6000_c000, "UHCI1"),
    (0x6000_f000, "I2S0"),
    (0x6001_0000, "UART1"),
    (0x6001_1000, "BT"),
    (0x6001_3000, "I2C0"),
    (0x6001_4000, "UHCI0"),
    (0x6001_5000, "SLCHOST"),
    (0x6001_6000, "RMT"),
    (0x6001_7000, "PCNT"),
    (0x6001_8000, "SLC"),
    (0x6001_9000, "LEDC"),
    (0x6001_cc00, "NRX"),
    (0x6001_d000, "BB"),
    (0x6001_e000, "MCPWM0"),
    (0x6001_f000, "TIMG0"),
    (0x6002_0000, "TIMG1"),
    (0x6002_1000, "RTC_SLOWMEM"),
    (0x6002_3000, "SYSTIMER"),
    (0x6002_4000, "SPI2"),
    (0x6002_5000, "SPI3"),
    (0x6002_6000, "SYSCON"),
    (0x6002_7000, "I2C1"),
    (0x6002_8000, "SDMMC"),
    (0x6002_a000, "PERI_BACKUP"),
    (0x6002_b000, "TWAI"),
    (0x6002_c000, "MCPWM1"),
    (0x6002_d000, "I2S1"),
    (0x6002_e000, "UART2"),
    (0x6003_8000, "USB_SERIAL_JTAG"),
    (0x6003_9000, "USB_WRAP"),
    (0x6003_a000, "AES"),
    (0x6003_b000, "SHA"),
    (0x6003_c000, "RSA"),
    (0x6003_d000, "DIGITAL_SIGNATURE"),
    (0x6003_e000, "HMAC"),
    (0x6003_f000, "GDMA"),
    (0x6004_0000, "APB_SARADC"),
    (0x6004_1000, "LCD_CAM"),
    (0x600c_0000, "SYSTEM"),
    (0x600c_1000, "SENSITIVE"),
    (0x600c_2000, "INTERRUPT"),
    (0x600c_4000, "EXTMEM"),
    (0x600c_5000, "MMU_TABLE"),
    (0x600c_6000, "ITAG_TABLE"),
    (0x600c_8000, "DTAG_TABLE"),
    (0x600c_c000, "EXT_MEM_ENC"),
    (0x600c_e000, "ASSIST_DEBUG"),
    (0x600d_0000, "WCL"),
];

/// `BLOCK+0x1c` for the block at or below `addr`, `PERIPHERAL+0x...` below every block.
pub(crate) fn name(addr: u32) -> String {
    let k = BLOCKS.partition_point(|&(base, _)| base <= addr);
    match k.checked_sub(1).map(|k| BLOCKS[k]) {
        Some((base, block)) => format!("{block}+{:#x}", addr - base),
        None => format!("PERIPHERAL+{addr:#x}"),
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
    }
}
