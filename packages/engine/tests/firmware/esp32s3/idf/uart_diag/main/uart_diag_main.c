// A board diagnostic for the UART0 probe in ../uart. No circuit runs it.
//
// Phase 1 waits for 'g' on the USB Serial/JTAG console, then runs the probe's sequence
// unchanged: route_pad, enable_uart0, both bauds, every case and the burst. It only adds
// loads of UART0's FIFO pointer and status registers, and prints them. Phase 2 waits for a
// second 'g', runs enable_uart0 again, sets the 115200 8N1 frame, pulses RXFIFO_RST and then
// TXFIFO_RST as IDF's uart_param_config does after the clock and the frame, and reruns the
// same cases with the same prints.
//
// Each snapshot is one line:
//   diag <point> txs <hex> w <n> r <n> rxs <hex> r <n> w <n> st <hex> txc <n> rxc <n> mem <hex> txsz <n> rxsz <n>
// txs is UART_MEM_TX_STATUS_REG: w is APB_TX_WADDR, r is TX_RADDR. rxs is
// UART_MEM_RX_STATUS_REG: r is APB_RX_RADDR, w is RX_WADDR. st is UART_STATUS_REG: txc is
// TXFIFO_CNT, rxc is RXFIFO_CNT. mem is UART_MEM_CONF_REG: txsz is TX_SIZE, rxsz is RX_SIZE.
// None of these loads has a side effect. The probe's own UART_FIFO_REG load stays the only one.
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "driver/usb_serial_jtag.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "soc/gpio_reg.h"
#include "soc/gpio_sig_map.h"
#include "soc/io_mux_reg.h"
#include "soc/system_reg.h"
#include "soc/uart_reg.h"

#define TX_PAD 43
#define U0_SIGNAL U0TXD_OUT_IDX
#define EDGE_CAP 24
#define EDGE_POLL_BOUND 64
#define FRAME_BOUND 40000
#define BURST_COUNT 8
#define BURST_BOUND (8 * 30000)

static void write_all(const char *text)
{
    size_t left = strlen(text);
    while (left > 0) {
        int wrote = usb_serial_jtag_write_bytes(text, left, portMAX_DELAY);
        if (wrote <= 0) {
            return;
        }
        text += wrote;
        left -= (size_t)wrote;
    }
}

static void print_line(const char *text)
{
    char line[48];
    snprintf(line, sizeof line, "%s\n", text);
    write_all(line);
}

static void wait_g(void)
{
    uint8_t byte = 0;
    while (usb_serial_jtag_read_bytes(&byte, 1, portMAX_DELAY) != 1 || byte != 'g') {
    }
}

// Loads the four words, then prints them. `point` names where in the sequence they were read.
static void snapshot(const char *point)
{
    char line[200];
    uint32_t txs = REG_READ(UART_MEM_TX_STATUS_REG(0));
    uint32_t rxs = REG_READ(UART_MEM_RX_STATUS_REG(0));
    uint32_t st = REG_READ(UART_STATUS_REG(0));
    uint32_t mem = REG_READ(UART_MEM_CONF_REG(0));
    snprintf(line, sizeof line,
             "diag %s txs %08lx w %lu r %lu rxs %08lx r %lu w %lu st %08lx txc %lu rxc %lu mem %08lx txsz %lu rxsz %lu\n",
             point, (unsigned long)txs, (unsigned long)((txs >> UART_APB_TX_WADDR_S) & UART_APB_TX_WADDR_V),
             (unsigned long)((txs >> UART_TX_RADDR_S) & UART_TX_RADDR_V), (unsigned long)rxs,
             (unsigned long)((rxs >> UART_APB_RX_RADDR_S) & UART_APB_RX_RADDR_V),
             (unsigned long)((rxs >> UART_RX_WADDR_S) & UART_RX_WADDR_V), (unsigned long)st,
             (unsigned long)((st >> UART_TXFIFO_CNT_S) & UART_TXFIFO_CNT_V),
             (unsigned long)((st >> UART_RXFIFO_CNT_S) & UART_RXFIFO_CNT_V), (unsigned long)mem,
             (unsigned long)((mem >> UART_TX_SIZE_S) & UART_TX_SIZE_V),
             (unsigned long)((mem >> UART_RX_SIZE_S) & UART_RX_SIZE_V));
    write_all(line);
}

static uint32_t ccount(void)
{
    uint32_t now;
    __asm__ volatile("rsr.ccount %0" : "=r"(now));
    return now;
}

static void route_pad(void)
{
    PIN_FUNC_SELECT(IO_MUX_GPIO43_REG, FUNC_U0TXD_GPIO43);
    PIN_INPUT_ENABLE(IO_MUX_GPIO43_REG);
    REG_WRITE(GPIO_FUNC0_OUT_SEL_CFG_REG + 4 * TX_PAD, U0_SIGNAL);
    REG_WRITE(GPIO_FUNC0_IN_SEL_CFG_REG + 4 * U0_SIGNAL, GPIO_SIG0_IN_SEL | TX_PAD);
}

// The probe's reset, unchanged.
static void enable_uart0(void)
{
    REG_SET_BIT(SYSTEM_PERIP_CLK_EN0_REG, SYSTEM_UART_CLK_EN);
    REG_CLR_BIT(SYSTEM_PERIP_RST_EN0_REG, SYSTEM_UART_RST);
    REG_SET_BIT(UART_CLK_CONF_REG(0), UART_RST_CORE);
    REG_SET_BIT(SYSTEM_PERIP_RST_EN0_REG, SYSTEM_UART_RST);
    REG_CLR_BIT(SYSTEM_PERIP_RST_EN0_REG, SYSTEM_UART_RST);
    REG_CLR_BIT(UART_CLK_CONF_REG(0), UART_RST_CORE);
}

// uart_param_config resets the RX FIFO, then the TX FIFO, each as a set and a clear of its
// CONF0 bit: uart_ll_rxfifo_rst and uart_ll_txfifo_rst.
static void reset_fifos(void)
{
    REG_SET_BIT(UART_CONF0_REG(0), UART_RXFIFO_RST);
    REG_CLR_BIT(UART_CONF0_REG(0), UART_RXFIFO_RST);
    REG_SET_BIT(UART_CONF0_REG(0), UART_TXFIFO_RST);
    REG_CLR_BIT(UART_CONF0_REG(0), UART_TXFIFO_RST);
}

static void set_clock(uint32_t div, uint32_t frag)
{
    REG_WRITE(UART_CLK_CONF_REG(0), (1 << UART_SCLK_SEL_S) | UART_SCLK_EN | UART_TX_SCLK_EN | UART_RX_SCLK_EN);
    REG_WRITE(UART_CLKDIV_REG(0), div | (frag << UART_CLKDIV_FRAG_S));
    REG_WRITE(UART_IDLE_CONF_REG(0), 256);
}

static void set_format(uint32_t conf0)
{
    REG_WRITE(UART_CONF0_REG(0), conf0);
}

static uint32_t conf0_of(const char *cfg)
{
    uint32_t word = UART_MEM_CLK_EN | (3 << UART_BIT_NUM_S) | (1 << UART_STOP_BIT_NUM_S);
    if (strcmp(cfg, "8E1") == 0) {
        word |= UART_PARITY_EN;
    }
    if (strcmp(cfg, "8N2") == 0) {
        word = UART_MEM_CLK_EN | (3 << UART_BIT_NUM_S) | (3 << UART_STOP_BIT_NUM_S);
    }
    return word;
}

static void send_byte(uint8_t byte, uint32_t edges[][2], int *nedges)
{
    uint32_t origin;
    uint32_t level;
    uint32_t now;
    *nedges = 0;
    portDISABLE_INTERRUPTS();
    __asm__ volatile("rsr.ccount %[o]\n"
                     "s32i %[b], %[f], 0\n"
                     : [o] "=&r"(origin)
                     : [b] "r"((uint32_t)byte), [f] "r"(UART_FIFO_REG(0))
                     : "memory");
    level = 1;
    while (*nedges < EDGE_CAP) {
        uint32_t seen = (REG_READ(GPIO_IN1_REG) >> (TX_PAD - 32)) & 1;
        now = ccount();
        if (now - origin > FRAME_BOUND) {
            break;
        }
        if (seen == level) {
            continue;
        }
        edges[*nedges][0] = now - origin;
        edges[*nedges][1] = seen;
        *nedges += 1;
        level = seen;
    }
    portENABLE_INTERRUPTS();
}

static int read_back(uint8_t *byte)
{
    uint32_t status = REG_READ(UART_STATUS_REG(0));
    if ((status & UART_RXFIFO_CNT_M) == 0) {
        return 0;
    }
    *byte = (uint8_t)REG_READ(UART_FIFO_REG(0));
    return 1;
}

// The probe's burst, with a snapshot after the edges and one after the read-backs.
static void send_burst(const uint8_t *bytes, int count)
{
    char line[48];
    uint32_t edges[EDGE_CAP][2];
    uint32_t origin;
    uint32_t level;
    uint32_t now;
    int nedges = 0;
    int i;
    uint8_t got = 0;
    portDISABLE_INTERRUPTS();
    __asm__ volatile("rsr.ccount %[o]\n"
                     "s32i %[b0], %[f], 0\n"
                     "s32i %[b1], %[f], 0\n"
                     "s32i %[b2], %[f], 0\n"
                     "s32i %[b3], %[f], 0\n"
                     "s32i %[b4], %[f], 0\n"
                     "s32i %[b5], %[f], 0\n"
                     "s32i %[b6], %[f], 0\n"
                     "s32i %[b7], %[f], 0\n"
                     : [o] "=&r"(origin)
                     : [b0] "r"((uint32_t)bytes[0]), [b1] "r"((uint32_t)bytes[1]),
                       [b2] "r"((uint32_t)bytes[2]), [b3] "r"((uint32_t)bytes[3]),
                       [b4] "r"((uint32_t)bytes[4]), [b5] "r"((uint32_t)bytes[5]),
                       [b6] "r"((uint32_t)bytes[6]), [b7] "r"((uint32_t)bytes[7]),
                       [f] "r"(UART_FIFO_REG(0))
                     : "memory");
    level = 1;
    while (nedges < EDGE_CAP) {
        uint32_t seen = (REG_READ(GPIO_IN1_REG) >> (TX_PAD - 32)) & 1;
        now = ccount();
        if (now - origin > BURST_BOUND) {
            break;
        }
        if (seen == level) {
            continue;
        }
        edges[nedges][0] = now - origin;
        edges[nedges][1] = seen;
        nedges += 1;
        level = seen;
    }
    portENABLE_INTERRUPTS();
    snapshot("burst");
    print_line("burst");
    for (i = 0; i < nedges; i++) {
        snprintf(line, sizeof line, "edge %lu %lu", (unsigned long)edges[i][0], (unsigned long)edges[i][1]);
        print_line(line);
    }
    for (i = 0; i < count; i++) {
        int have = read_back(&got);
        snprintf(line, sizeof line, "tx %02x", bytes[i]);
        print_line(line);
        if (have) {
            snprintf(line, sizeof line, "rx %02x", got);
        } else {
            snprintf(line, sizeof line, "rx none");
        }
        print_line(line);
    }
    snapshot("burst_rx");
}

// The probe's byte, with a snapshot after its read-back.
static void one_byte(uint8_t byte)
{
    char line[48];
    char point[16];
    uint32_t edges[EDGE_CAP][2];
    int nedges = 0;
    uint8_t got = 0;
    int have;
    int i;
    send_byte(byte, edges, &nedges);
    have = read_back(&got);
    snprintf(point, sizeof point, "tx_%02x", byte);
    snapshot(point);
    snprintf(line, sizeof line, "tx %02x", byte);
    print_line(line);
    if (have) {
        snprintf(line, sizeof line, "rx %02x", got);
    } else {
        snprintf(line, sizeof line, "rx none");
    }
    print_line(line);
    for (i = 0; i < nedges; i++) {
        snprintf(line, sizeof line, "edge %lu %lu", (unsigned long)edges[i][0], (unsigned long)edges[i][1]);
        print_line(line);
    }
}

static void one_case(const char *cfg, const uint8_t *bytes, int count)
{
    char line[48];
    char point[16];
    int i;
    set_format(conf0_of(cfg));
    snprintf(point, sizeof point, "format_%s", cfg);
    snapshot(point);
    snprintf(line, sizeof line, "cfg %s", cfg);
    print_line(line);
    for (i = 0; i < count; i++) {
        one_byte(bytes[i]);
    }
}

static void one_baud(uint32_t baud, uint32_t div, uint32_t frag)
{
    static const uint8_t n1[] = {0x00, 0x55, 0xff};
    static const uint8_t e1[] = {0x01};
    static const uint8_t n2[] = {0xa5};
    char line[48];
    char point[24];
    snprintf(line, sizeof line, "baud %lu", (unsigned long)baud);
    print_line(line);
    static const uint8_t burst[BURST_COUNT] = {0, 0, 0, 0, 0, 0, 0, 0};
    set_clock(div, frag);
    snprintf(point, sizeof point, "clock_%lu", (unsigned long)baud);
    snapshot(point);
    one_case("8N1", n1, 3);
    one_case("8E1", e1, 1);
    one_case("8N2", n2, 1);
    if (frag != 0) {
        set_format(conf0_of("8N1"));
        snapshot("format_burst");
        send_burst(burst, BURST_COUNT);
    }
}

static void both_bauds(void)
{
    one_baud(115200, 694, 7);
    one_baud(1000000, 80, 0);
}

void app_main(void)
{
    usb_serial_jtag_driver_config_t config = USB_SERIAL_JTAG_DRIVER_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&config));

    wait_g();
    route_pad();
    // UART0's clock is off and its reset held here, as esp_perip_clk_init leaves it. The line
    // goes out first, so a load that never returns still shows where it stopped.
    print_line("diag pre loads");
    usb_serial_jtag_wait_tx_done(portMAX_DELAY);
    snapshot("pre");
    enable_uart0();
    snapshot("enable");
    both_bauds();
    print_line("UART DIAG 1 DONE");

    wait_g();
    enable_uart0();
    snapshot("enable");
    set_clock(694, 7);
    set_format(conf0_of("8N1"));
    snapshot("config");
    reset_fifos();
    snapshot("fifo_rst");
    both_bauds();
    print_line("UART DIAG 2 DONE");
    vTaskDelay(portMAX_DELAY);
}
