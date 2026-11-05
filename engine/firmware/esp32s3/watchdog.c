/* Arms timer group 0's watchdog, stops feeding it, and prints on the USB Serial/JTAG console
 * when each stage runs out. Bare metal: no boot ROM call, no IDF, no flash, and no interrupt.
 * watchdog_start.S holds the vectors.
 *
 * It first prints SYSTEM_SYSCLK_CONF_REG and SYSTEM_CPU_PER_CONF_REG, which say what clocks the
 * CPU and the APB. On the 40 MHz crystal both run at 40 MHz, so a prescaler of 40000 makes the
 * watchdog count once a millisecond. Stage 0 holds 100 counts and interrupts, and stage 1 holds
 * 50 and resets the system, 150 ms after the store that arms the watchdog. The program reads
 * SYSTIMER, which counts 16 a microsecond, just after that store. It polls stage 0's bit of
 * TIMG_INT_RAW_TIMERS_REG and prints the microseconds since then. From 148 ms on it prints
 * the microseconds again every 100 us, until the reset.
 *
 * The S3's watchdog has no reset for its prescaler's count, so its first count can come up to
 * one count, 1 ms, early or late, and both stages move with it. A board passes when:
 *   - stage0 prints 99000 to 101100, which leaves room for the poll that reads the stage's bit;
 *   - the last us line before the reset prints 148800 to 151000, since the reset can drop the
 *     line in flight.
 * Each us line's target is 148000 plus 100 for each line before it. A line more than 20 us past
 * its target means the host fell behind, not the watchdog, and the board is captured again, as
 * for a line missing before the last. The model starts the count at the arming store.
 *
 *   xtensa-esp32s3-elf-gcc -mabi=call0 -O2 -g0 -ffreestanding -nostdlib -nostartfiles \
 *       -fno-builtin -Wall -Wextra -Werror -Wl,-T,ram.ld -Wl,--build-id=none -Wl,-n \
 *       -o watchdog.elf watchdog_start.S watchdog.c
 *
 * In the model the console prints exactly this, and the run then ends with a timer group 0
 * system reset 150 ms after the arming store:
 *
 * EXPECTED-BEGIN
 * BEGIN watchdog
 * sysclk_conf 0x00000000
 * cpu_per_conf 0x0000000c
 * prescaler 40000
 * holds 100 50
 * stage0 100000
 * us 148000
 * us 148100
 * ...
 * us 149900
 * EXPECTED-END
 *
 * with every 100 us from 148000 to 149900 on its own line. A sysclk_conf whose bits 11:10 and
 * 9:0 are 0 is the crystal, undivided. On the crystal the APB runs at the CPU clock, 40 MHz over
 * bits 9:0 plus 1, so a sysclk_conf ending in 0x001 is 20 MHz, and stage0 prints about 200000.
 */

#include <stdint.h>

#define REG(addr) (*(volatile uint32_t *)(addr))

#define TIMG0_BASE 0x6001f000u
#define TIMG1_BASE 0x60020000u
#define TIMG_WDTCONFIG(base, n) REG((base) + 0x48 + 4 * (n))
#define TIMG_WDTWPROTECT(base) REG((base) + 0x64)
#define TIMG_INT_RAW(base) REG((base) + 0x74)
#define TIMG_INT_CLR(base) REG((base) + 0x7c)
#define WDT_INT (1u << 2)
#define WDT_EN (1u << 31)
#define STG0_INT (1u << 29)
#define STG1_RESET_SYSTEM (3u << 27)
#define RESET_LENGTHS (7u << 18 | 7u << 15)
#define RTC_CNTL_BASE 0x60008000u
#define RTC_CNTL_WDTCONFIG0 REG(RTC_CNTL_BASE + 0x98)
#define RTC_CNTL_WDTWPROTECT REG(RTC_CNTL_BASE + 0xb0)
#define RTC_CNTL_SWD_CONF REG(RTC_CNTL_BASE + 0xb4)
#define RTC_CNTL_SWD_WPROTECT REG(RTC_CNTL_BASE + 0xb8)
#define WDT_KEY 0x50d83aa1u
#define SWD_KEY 0x8f1d312au
#define SWD_DISABLE (1u << 30)

#define SYSTEM_CPU_PER_CONF REG(0x600c0010u)
#define SYSTEM_SYSCLK_CONF REG(0x600c0060u)

#define SYSTIMER_BASE 0x60023000u
#define SYSTIMER_UNIT0_OP REG(SYSTIMER_BASE + 0x04)
#define SYSTIMER_UNIT0_VALUE_LO REG(SYSTIMER_BASE + 0x44)
#define UPDATE (1u << 30)
#define VALUE_VALID (1u << 29)
#define PER_US 16u

#define USB_BASE 0x60038000u
#define USB_EP1 REG(USB_BASE + 0x00)
#define USB_EP1_CONF REG(USB_BASE + 0x04)
#define WR_DONE (1u << 0)
#define SERIAL_IN_EP_DATA_FREE (1u << 1)

#define PRESCALER 40000u
#define STAGE0 100u
#define STAGE1 50u
#define FINE_FROM_US 148000u
#define FINE_EVERY_US 100u

static void put(char c) {
    while (!(USB_EP1_CONF & SERIAL_IN_EP_DATA_FREE)) {
    }
    USB_EP1 = (uint8_t)c;
}

static void puts_raw(const char *s) {
    while (*s) {
        put(*s++);
    }
}

static void dec(uint32_t value) {
    char digits[10];
    int n = 0;
    do {
        digits[n++] = (char)('0' + value % 10);
        value /= 10;
    } while (value);
    while (n) {
        put(digits[--n]);
    }
}

static void line(const char *label, uint32_t value) {
    puts_raw(label);
    put(' ');
    dec(value);
    put('\n');
    USB_EP1_CONF = WR_DONE;
}

static void hex_line(const char *label, uint32_t value) {
    puts_raw(label);
    puts_raw(" 0x");
    for (int shift = 28; shift >= 0; shift -= 4) {
        put("0123456789abcdef"[(value >> shift) & 0xf]);
    }
    put('\n');
    USB_EP1_CONF = WR_DONE;
}

static uint32_t systimer(void) {
    SYSTIMER_UNIT0_OP = UPDATE;
    while (!(SYSTIMER_UNIT0_OP & VALUE_VALID)) {
    }
    return SYSTIMER_UNIT0_VALUE_LO;
}

static void disable_other_watchdogs(void) {
    TIMG_WDTWPROTECT(TIMG1_BASE) = WDT_KEY;
    TIMG_WDTCONFIG(TIMG1_BASE, 0) = 0;
    TIMG_WDTWPROTECT(TIMG1_BASE) = 0;
    RTC_CNTL_WDTWPROTECT = WDT_KEY;
    RTC_CNTL_WDTCONFIG0 = 0;
    RTC_CNTL_WDTWPROTECT = 0;
    RTC_CNTL_SWD_WPROTECT = SWD_KEY;
    RTC_CNTL_SWD_CONF |= SWD_DISABLE;
    RTC_CNTL_SWD_WPROTECT = 0;
}

void main(void) {
    disable_other_watchdogs();
    puts_raw("BEGIN watchdog\n");
    USB_EP1_CONF = WR_DONE;
    hex_line("sysclk_conf", SYSTEM_SYSCLK_CONF);
    hex_line("cpu_per_conf", SYSTEM_CPU_PER_CONF);
    line("prescaler", PRESCALER);
    puts_raw("holds ");
    dec(STAGE0);
    put(' ');
    dec(STAGE1);
    put('\n');
    USB_EP1_CONF = WR_DONE;

    TIMG_WDTWPROTECT(TIMG0_BASE) = WDT_KEY;
    TIMG_WDTCONFIG(TIMG0_BASE, 0) = 0;
    TIMG_INT_CLR(TIMG0_BASE) = WDT_INT;
    TIMG_WDTCONFIG(TIMG0_BASE, 1) = PRESCALER << 16;
    TIMG_WDTCONFIG(TIMG0_BASE, 2) = STAGE0;
    TIMG_WDTCONFIG(TIMG0_BASE, 3) = STAGE1;
    TIMG_WDTCONFIG(TIMG0_BASE, 0) = WDT_EN | STG0_INT | STG1_RESET_SYSTEM | RESET_LENGTHS;
    uint32_t armed = systimer();
    TIMG_WDTWPROTECT(TIMG0_BASE) = 0;

    while (!(TIMG_INT_RAW(TIMG0_BASE) & WDT_INT)) {
    }
    line("stage0", (systimer() - armed) / PER_US);
    for (uint32_t us = FINE_FROM_US;; us += FINE_EVERY_US) {
        uint32_t now;
        while ((now = systimer() - armed) < us * PER_US) {
        }
        line("us", now / PER_US);
    }
}
