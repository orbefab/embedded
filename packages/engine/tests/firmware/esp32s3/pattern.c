/* Draws pattern.json on an ST77916 panel over SPI2. Bare metal: no boot ROM
 * call, no IDF, no flash. pattern_start.S holds the vectors, and pattern.h, which
 * packages/engine/scripts/pattern_header.py writes from pattern.json, holds the init commands and the
 * rectangles.
 *
 * SPI2 runs from its 64-byte buffer at 10 MHz off the crystal, routed through the GPIO matrix:
 * SCLK on GPIO11, CS on GPIO12 and D0 to D3 on GPIO13 to GPIO16. RST is GPIO17 and the backlight
 * GPIO21. The program resets the panel, holding RST low for 10 ms and then waiting 120 ms, and
 * sends each init command and waits its delay. It turns the backlight on and writes each
 * rectangle: CASET and RASET, then RAMWR and the rectangle's pixels, RGB565 high byte first.
 *
 * Each command is the 32-bit word opcode << 24 | command << 8, sent as an 8-bit command phase
 * and a 24-bit address phase on one lane. A command with parameters is that word with CS kept,
 * then the parameters on one lane. A pixel write is the word with opcode 0x32 and CS kept, then
 * the pixels on four lanes in 64-byte transfers, with CS kept on all but the last.
 *
 * The console prints exactly this:
 *
 * EXPECTED-BEGIN
 * BEGIN pattern
 * END pattern
 * EXPECTED-END
 */

#include <stdint.h>

#include "pattern.h"

#define REG(addr) (*(volatile uint32_t *)(addr))

#define GPIO_BASE 0x60004000u
#define GPIO_OUT_W1TS REG(GPIO_BASE + 0x08)
#define GPIO_OUT_W1TC REG(GPIO_BASE + 0x0c)
#define GPIO_ENABLE_W1TS REG(GPIO_BASE + 0x24)
#define GPIO_FUNC_OUT_SEL_CFG(n) REG(GPIO_BASE + 0x554 + 4 * (n))
#define SIG_GPIO_OUT 0x100u

#define IO_MUX_BASE 0x60009000u
#define IO_MUX_GPIO(n) REG(IO_MUX_BASE + 0x04 + 4 * (n))
#define FUN_DRV_2 (2u << 10)
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

/* SPI2, IDF's GPSPI2, from soc/esp32s3/register/soc/spi_reg.h. */
#define SPI2_BASE 0x60024000u
#define SPI_CMD REG(SPI2_BASE + 0x00)
#define SPI_ADDR REG(SPI2_BASE + 0x04)
#define SPI_CLOCK REG(SPI2_BASE + 0x0c)
#define SPI_USER REG(SPI2_BASE + 0x10)
#define SPI_USER1 REG(SPI2_BASE + 0x14)
#define SPI_USER2 REG(SPI2_BASE + 0x18)
#define SPI_MS_DLEN REG(SPI2_BASE + 0x1c)
#define SPI_MISC REG(SPI2_BASE + 0x20)
#define SPI_DMA_INT_CLR REG(SPI2_BASE + 0x38)
#define SPI_DMA_INT_RAW REG(SPI2_BASE + 0x3c)
#define SPI_W(n) REG(SPI2_BASE + 0x98 + 4 * (n))
#define SPI_CLK_GATE REG(SPI2_BASE + 0xe8)
#define SPI_USR (1u << 24)
#define SPI_UPDATE (1u << 23)
#define SPI_USR_COMMAND (1u << 31)
#define SPI_USR_ADDR (1u << 30)
#define SPI_USR_MOSI (1u << 27)
#define SPI_FWRITE_QUAD (1u << 13)
#define SPI_ADDR_BITLEN_24 (23u << 27)
#define SPI_COMMAND_BITLEN_8 (7u << 28)
#define SPI_CS_KEEP_ACTIVE (1u << 30)
/* CS1 to CS5 off, as at reset. */
#define SPI_CS1_TO_CS5_DIS (0x1fu << 1)
#define SPI_TRANS_DONE (1u << 12)
#define SPI_CLK_EN (1u << 0)
/* Powers the module clock, as IDF's spi_ll_master_init sets it. */
#define SPI_MST_CLK_ACTIVE (1u << 1)
/* The crystal divided by 4: CLKCNT_N 3, CLKCNT_H 1, CLKCNT_L 3. */
#define SPI_CLOCK_10MHZ ((3u << 12) | (1u << 6) | 3u)

/* GPIO matrix output signals, from soc/esp32s3/include/soc/gpio_sig_map.h. */
#define FSPICLK_OUT 101u
#define FSPIQ_OUT 102u
#define FSPID_OUT 103u
#define FSPIHD_OUT 104u
#define FSPIWP_OUT 105u
#define FSPICS0_OUT 110u

#define PIN_SCLK 11
#define PIN_CS 12
#define PIN_D0 13
#define PIN_D1 14
#define PIN_D2 15
#define PIN_D3 16
#define PIN_RST 17
#define PIN_BLK 21

#define CYCLES_PER_MS 40000u
#define CCOMPARE1_INTERRUPT 15
#define BUFFER 64

#define OPCODE_REGISTER 0x02
#define OPCODE_PIXELS 0x32
#define CASET 0x2a
#define RASET 0x2b
#define RAMWR 0x2c

static union {
    uint8_t bytes[BUFFER];
    uint32_t words[BUFFER / 4];
} buffer;

static void put(char c) {
    while (!(USB_EP1_CONF & SERIAL_IN_EP_DATA_FREE)) {
    }
    USB_EP1 = (uint8_t)c;
}

/* One line, sent as one packet. */
static void line(const char *s) {
    while (*s) {
        put(*s++);
    }
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

static uint32_t ccount(void) {
    uint32_t now;
    __asm__ volatile("rsr %0, ccount" : "=r"(now));
    return now;
}

/* Sleeps in WAITI until CCOMPARE1 matches. The level-3 vector only clears the match. */
static void delay_ms(uint32_t ms) {
    if (ms == 0) {
        return;
    }
    uint32_t until = ccount() + ms * CYCLES_PER_MS;
    __asm__ volatile("wsr %0, ccompare1" : : "r"(until));
    uint32_t enable = 1u << CCOMPARE1_INTERRUPT;
    __asm__ volatile("wsr %0, intenable" : : "r"(enable));
    while ((int32_t)(ccount() - until) < 0) {
        __asm__ volatile("waiti 0" : : : "memory");
    }
    __asm__ volatile("wsr %0, intenable" : : "r"(0));
}

static void route(int pin, uint32_t signal) {
    IO_MUX_GPIO(pin) = MCU_SEL_GPIO | FUN_DRV_2;
    GPIO_FUNC_OUT_SEL_CFG(pin) = signal;
    GPIO_ENABLE_W1TS = 1u << pin;
}

/* Runs one transfer with `user` and the first `bytes` of the buffer, and waits for its end. */
static void transfer(uint32_t user, uint32_t bytes, int keep) {
    for (uint32_t k = 0; k < (bytes + 3) / 4; k++) {
        SPI_W(k) = buffer.words[k];
    }
    SPI_USER = user;
    SPI_MS_DLEN = bytes ? bytes * 8 - 1 : 0;
    SPI_MISC = SPI_CS1_TO_CS5_DIS | (keep ? SPI_CS_KEEP_ACTIVE : 0);
    SPI_CMD = SPI_UPDATE;
    while (SPI_CMD & SPI_UPDATE) {
    }
    SPI_DMA_INT_CLR = SPI_TRANS_DONE;
    SPI_CMD = SPI_USR;
    while (!(SPI_DMA_INT_RAW & SPI_TRANS_DONE)) {
    }
}

static void command(uint8_t opcode, uint8_t command, int keep) {
    SPI_USER2 = SPI_COMMAND_BITLEN_8 | opcode;
    SPI_ADDR = (uint32_t)command << 16;
    transfer(SPI_USR_COMMAND | SPI_USR_ADDR, 0, keep);
}

static void register_write(uint8_t command_byte, const uint8_t *params, uint32_t n) {
    command(OPCODE_REGISTER, command_byte, n != 0);
    if (n) {
        for (uint32_t k = 0; k < n; k++) {
            buffer.bytes[k] = params[k];
        }
        transfer(SPI_USR_MOSI, n, 0);
    }
}

static uint16_t rgb(uint32_t r, uint32_t g, uint32_t b) {
    return (uint16_t)((r & 31) << 11 | (g & 63) << 5 | (b & 31));
}

static uint16_t pixel(const struct rect *r, uint32_t x, uint32_t y) {
    uint32_t w = r->x1 - r->x0 + 1u, h = r->y1 - r->y0 + 1u;
    uint32_t dx = x - r->x0, dy = y - r->y0;
    switch (r->fill) {
    case FILL_BARS:
        return r->colors[dx * r->ncolors / w];
    case FILL_CHECKER:
        return r->colors[(dx / r->cell + dy / r->cell) & 1];
    case FILL_GRADIENT:
        return rgb(dx * 31 / (w - 1), dy * 63 / (h - 1), r->blue);
    default: {
        uint32_t v = (x ^ y) & 0xff;
        return rgb(v >> 3, v >> 2, 31 - (v >> 3));
    }
    }
}

static void window(uint8_t command_byte, uint16_t start, uint16_t end) {
    const uint8_t params[4] = {start >> 8, start & 0xff, end >> 8, end & 0xff};
    register_write(command_byte, params, 4);
}

static void rect_write(const struct rect *r) {
    window(CASET, r->x0, r->x1);
    window(RASET, r->y0, r->y1);
    command(OPCODE_PIXELS, RAMWR, 1);
    uint32_t total = (uint32_t)(r->x1 - r->x0 + 1u) * (r->y1 - r->y0 + 1u) * 2u;
    uint32_t sent = 0, fill = 0;
    for (uint32_t y = r->y0; y <= r->y1; y++) {
        for (uint32_t x = r->x0; x <= r->x1; x++) {
            uint16_t p = pixel(r, x, y);
            buffer.bytes[fill++] = p >> 8;
            buffer.bytes[fill++] = p & 0xff;
            if (fill == BUFFER || sent + fill == total) {
                sent += fill;
                transfer(SPI_USR_MOSI | SPI_FWRITE_QUAD, fill, sent != total);
                fill = 0;
            }
        }
    }
}

void main(void) {
    disable_watchdogs();
    line("BEGIN pattern");
    route(PIN_RST, SIG_GPIO_OUT);
    route(PIN_BLK, SIG_GPIO_OUT);
    GPIO_OUT_W1TC = 1u << PIN_BLK;
    GPIO_OUT_W1TS = 1u << PIN_RST;
    route(PIN_SCLK, FSPICLK_OUT);
    route(PIN_CS, FSPICS0_OUT);
    route(PIN_D0, FSPID_OUT);
    route(PIN_D1, FSPIQ_OUT);
    route(PIN_D2, FSPIWP_OUT);
    route(PIN_D3, FSPIHD_OUT);
    SPI_CLK_GATE = SPI_CLK_EN | SPI_MST_CLK_ACTIVE;
    SPI_CLOCK = SPI_CLOCK_10MHZ;
    SPI_USER1 = SPI_ADDR_BITLEN_24;

    GPIO_OUT_W1TC = 1u << PIN_RST;
    delay_ms(10);
    GPIO_OUT_W1TS = 1u << PIN_RST;
    delay_ms(120);
    const uint8_t *at = init_commands;
    for (int k = 0; k < INIT_COMMANDS; k++) {
        uint8_t n = at[1], delay = at[2];
        register_write(at[0], at + 3, n);
        delay_ms(delay);
        at += 3 + n;
    }
    GPIO_OUT_W1TS = 1u << PIN_BLK;
    for (int k = 0; k < RECTS; k++) {
        rect_write(&rects[k]);
    }
    line("END pattern");
}
