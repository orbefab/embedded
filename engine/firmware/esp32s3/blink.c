/* Blinks GPIO21 from the CCOMPARE1 interrupt and prints what it saw on the USB Serial/JTAG
 * console. Bare metal: no boot ROM call, no IDF, no flash. start.S holds the vectors and the
 * interrupt.
 *
 * The CPU runs on the 40 MHz crystal. Edge k, from 1, comes at cycle k * PERIOD plus the
 * interrupt's fixed latency. Each edge's `in` is GPIO21's level just before it, the level the
 * edge before left. SYSTIMER counts 16 MHz, so the last edge's stamp less the first's is
 * (EDGES - 1) * PERIOD * 2 / 5.
 *
 * The console prints exactly this:
 *
 * EXPECTED-BEGIN
 * BEGIN blink
 * pin 00000015
 * period 003d0900
 * edges 00000008
 * edge 00000001 in 00000000
 * edge 00000002 in 00000001
 * edge 00000003 in 00000000
 * edge 00000004 in 00000001
 * edge 00000005 in 00000000
 * edge 00000006 in 00000001
 * edge 00000007 in 00000000
 * edge 00000008 in 00000001
 * systimer 00aae600
 * END blink
 * EXPECTED-END
 */

#include <stdint.h>

#define REG(addr) (*(volatile uint32_t *)(addr))

#define GPIO_BASE 0x60004000u
#define GPIO_OUT_W1TC REG(GPIO_BASE + 0x0c)
#define GPIO_ENABLE_W1TS REG(GPIO_BASE + 0x24)
#define GPIO_FUNC_OUT_SEL_CFG(n) REG(GPIO_BASE + 0x554 + 4 * (n))
#define SIG_GPIO_OUT 0x100u

#define IO_MUX_BASE 0x60009000u
#define IO_MUX_GPIO(n) REG(IO_MUX_BASE + 0x04 + 4 * (n))
#define FUN_IE (1u << 9)
#define FUN_DRV_3 (3u << 10)
#define MCU_SEL_GPIO (1u << 12)

#define TIMG0_BASE 0x6001f000u
#define TIMG1_BASE 0x60020000u
#define TIMG_WDTCONFIG0(base) REG((base) + 0x48)
#define TIMG_WDTWPROTECT(base) REG((base) + 0x64)
#define RTC_CNTL_BASE 0x60008000u
#define RTC_CNTL_WDTCONFIG0 REG(RTC_CNTL_BASE + 0x98)
#define RTC_CNTL_WDTWPROTECT REG(RTC_CNTL_BASE + 0xb0)
#define RTC_CNTL_SWD_CONF REG(RTC_CNTL_BASE + 0xb4)
#define RTC_CNTL_SWD_WPROTECT REG(RTC_CNTL_BASE + 0xb8)
#define WDT_KEY 0x50d83aa1u
#define SWD_KEY 0x8f1d312au
#define SWD_DISABLE (1u << 30)

#define USB_BASE 0x60038000u
#define USB_EP1 REG(USB_BASE + 0x00)
#define USB_EP1_CONF REG(USB_BASE + 0x04)
#define WR_DONE (1u << 0)
#define SERIAL_IN_EP_DATA_FREE (1u << 1)

#define LED 21
#define EDGES 8
#define CCOMPARE1_INTERRUPT 15

/* Written by the interrupt: how many edges it made, then each edge's level before it. */
volatile uint32_t edges[1 + EDGES];
/* SYSTIMER's low word just after each edge. */
volatile uint32_t stamps[EDGES];

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

static void hex(uint32_t value) {
    for (int shift = 28; shift >= 0; shift -= 4) {
        put("0123456789abcdef"[(value >> shift) & 0xf]);
    }
}

/* One line, sent as one packet. */
static void line(const char *label, uint32_t value) {
    puts_raw(label);
    put(' ');
    hex(value);
    put('\n');
    USB_EP1_CONF = WR_DONE;
}

static void disable_watchdogs(void) {
    TIMG_WDTWPROTECT(TIMG0_BASE) = WDT_KEY;
    TIMG_WDTCONFIG0(TIMG0_BASE) = 0;
    TIMG_WDTWPROTECT(TIMG0_BASE) = 0;
    TIMG_WDTWPROTECT(TIMG1_BASE) = WDT_KEY;
    TIMG_WDTCONFIG0(TIMG1_BASE) = 0;
    TIMG_WDTWPROTECT(TIMG1_BASE) = 0;
    RTC_CNTL_WDTWPROTECT = WDT_KEY;
    RTC_CNTL_WDTCONFIG0 = 0;
    RTC_CNTL_WDTWPROTECT = 0;
    RTC_CNTL_SWD_WPROTECT = SWD_KEY;
    RTC_CNTL_SWD_CONF |= SWD_DISABLE;
    RTC_CNTL_SWD_WPROTECT = 0;
}

void main(void) {
    disable_watchdogs();
    IO_MUX_GPIO(LED) = MCU_SEL_GPIO | FUN_DRV_3 | FUN_IE;
    GPIO_FUNC_OUT_SEL_CFG(LED) = SIG_GPIO_OUT;
    GPIO_OUT_W1TC = 1u << LED;
    GPIO_ENABLE_W1TS = 1u << LED;

    puts_raw("BEGIN blink\n");
    USB_EP1_CONF = WR_DONE;
    line("pin", LED);
    line("period", PERIOD);

    uint32_t first = PERIOD;
    __asm__ volatile("wsr %0, ccompare1" : : "r"(first));
    uint32_t enable = 1u << CCOMPARE1_INTERRUPT;
    __asm__ volatile("wsr %0, intenable" : : "r"(enable));
    while (edges[0] < EDGES) {
        __asm__ volatile("waiti 0" : : : "memory");
    }
    __asm__ volatile("wsr %0, intenable" : : "r"(0));

    line("edges", edges[0]);
    for (uint32_t k = 1; k <= EDGES; k++) {
        puts_raw("edge ");
        hex(k);
        line(" in", edges[k]);
    }
    line("systimer", stamps[EDGES - 1] - stamps[0]);
    puts_raw("END blink\n");
    USB_EP1_CONF = WR_DONE;
}
