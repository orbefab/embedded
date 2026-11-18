// Waits for 'g' on the USB Serial/JTAG console, then loops UART0 through GPIO43.
// TX is the pad. The pad's input is routed back to U0RXD, with FUN_IE set, so each
// byte returns through the pad with nothing wired to it. The checker reads the
// `checker:` lines.
//
// checker: APB_HZ 80000000
// checker: CPU_HZ 240000000
// checker: EDGE_POLL_BOUND 64
// checker: FRAME_BOUND 40000
// checker: BAUD 115200 DIV 694 FRAG 7
// checker: BAUD 1000000 DIV 80 FRAG 0
// checker: CASE 8N1 0x00 0x55 0xff
// checker: CASE 8E1 0x01
// checker: CASE 8N2 0xa5
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

// The USB console gates UART0. The same sequence `uart_ll_enable_bus_clock` and
// `uart_ll_reset_register` use puts the port back where a FIFO write shifts.
static void enable_uart0(void)
{
    REG_SET_BIT(SYSTEM_PERIP_CLK_EN0_REG, SYSTEM_UART_CLK_EN);
    REG_CLR_BIT(SYSTEM_PERIP_RST_EN0_REG, SYSTEM_UART_RST);
    REG_SET_BIT(UART_CLK_CONF_REG(0), UART_RST_CORE);
    REG_SET_BIT(SYSTEM_PERIP_RST_EN0_REG, SYSTEM_UART_RST);
    REG_CLR_BIT(SYSTEM_PERIP_RST_EN0_REG, SYSTEM_UART_RST);
    REG_CLR_BIT(UART_CLK_CONF_REG(0), UART_RST_CORE);
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
        // `now` is after the load that saw the edge, so the offset is not earlier than the edge.
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

static void one_byte(uint8_t byte)
{
    char line[48];
    uint32_t edges[EDGE_CAP][2];
    int nedges = 0;
    uint8_t got = 0;
    int have;
    int i;
    send_byte(byte, edges, &nedges);
    have = read_back(&got);
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
    int i;
    set_format(conf0_of(cfg));
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
    snprintf(line, sizeof line, "baud %lu", (unsigned long)baud);
    print_line(line);
    set_clock(div, frag);
    one_case("8N1", n1, 3);
    one_case("8E1", e1, 1);
    one_case("8N2", n2, 1);
}

void app_main(void)
{
    uint8_t byte = 0;
    usb_serial_jtag_driver_config_t config = USB_SERIAL_JTAG_DRIVER_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&config));
    while (usb_serial_jtag_read_bytes(&byte, 1, portMAX_DELAY) != 1 || byte != 'g') {
    }
    route_pad();
    enable_uart0();
    one_baud(115200, 694, 7);
    one_baud(1000000, 80, 0);
    print_line("UART DONE");
    vTaskDelay(portMAX_DELAY);
}
