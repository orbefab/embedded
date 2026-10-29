/*
 * Link-time wraps that log each panel transfer and SPI transaction. Every line goes out through cap_write, one line per event:
 *
 *   P <seq> <cmd> <len> <params hex> <us>              esp_lcd_panel_io_tx_param
 *   C <seq> <cmd> <len> <hash> <us>                    esp_lcd_panel_io_tx_color
 *   B <seq> <base64>                                   that transfer's bytes, when cap_dump is set
 *   F <seq> <x0> <y0> <x1> <y1> <len> <hash> <us>      esp_lcd_panel_draw_bitmap, ends inclusive
 *   S <seq> <api> <flags> <cmd_bits> <cmd> <addr_bits> <addr> <len_bits>    one SPI transaction
 *
 * <hash> is the first 8 bytes of SHA-256 in lowercase hex, the hash of display.csv and link.csv.
 * <seq> counts P, C and F lines together. S lines carry the seq of the transfer they belong to.
 * cmd_bits and addr_bits are "-" when the transaction uses the device default.
 */
#include <stdio.h>
#include <string.h>
#include "driver/spi_master.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_timer.h"
#include "mbedtls/base64.h"
#include "mbedtls/sha256.h"

void cap_write(const char *s, size_t n);

volatile int cap_dump = 0;
static uint32_t seq = 0;
static char line[4200];

esp_err_t __real_esp_lcd_panel_io_tx_param(esp_lcd_panel_io_handle_t io, int cmd, const void *param, size_t len);
esp_err_t __real_esp_lcd_panel_io_tx_color(esp_lcd_panel_io_handle_t io, int cmd, const void *color, size_t len);
esp_err_t __real_esp_lcd_panel_draw_bitmap(esp_lcd_panel_handle_t panel, int x0, int y0, int x1, int y1, const void *color);
esp_err_t __real_spi_device_queue_trans(spi_device_handle_t h, spi_transaction_t *t, TickType_t wait);
esp_err_t __real_spi_device_polling_transmit(spi_device_handle_t h, spi_transaction_t *t);
esp_err_t __real_spi_device_transmit(spi_device_handle_t h, spi_transaction_t *t);

static void hash16(const void *data, size_t len, char out[17])
{
    unsigned char d[32];
    mbedtls_sha256(data, len, d, 0);
    for (int i = 0; i < 8; i++) {
        snprintf(out + 2 * i, 3, "%02x", d[i]);
    }
}

static void emit(int n)
{
    cap_write(line, (size_t)n);
}

static void dump(uint32_t s, const uint8_t *data, size_t len)
{
    const size_t chunk = 3072;
    for (size_t off = 0; off < len; off += chunk) {
        size_t take = len - off < chunk ? len - off : chunk;
        int n = snprintf(line, sizeof line, "B %lu ", (unsigned long)s);
        size_t olen = 0;
        mbedtls_base64_encode((unsigned char *)line + n, sizeof line - n - 2, &olen, data + off, take);
        n += (int)olen;
        line[n++] = '\n';
        emit(n);
    }
}

esp_err_t __wrap_esp_lcd_panel_io_tx_param(esp_lcd_panel_io_handle_t io, int cmd, const void *param, size_t len)
{
    uint32_t s = ++seq;
    int n = snprintf(line, sizeof line, "P %lu %08x %u ", (unsigned long)s, (unsigned)cmd, (unsigned)len);
    const uint8_t *p = param;
    for (size_t i = 0; i < len && n < (int)sizeof line - 40; i++) {
        n += snprintf(line + n, sizeof line - n, "%02x", p[i]);
    }
    if (len == 0) {
        line[n++] = '-';
    }
    n += snprintf(line + n, sizeof line - n, " %lld\n", (long long)esp_timer_get_time());
    emit(n);
    return __real_esp_lcd_panel_io_tx_param(io, cmd, param, len);
}

esp_err_t __wrap_esp_lcd_panel_io_tx_color(esp_lcd_panel_io_handle_t io, int cmd, const void *color, size_t len)
{
    uint32_t s = ++seq;
    char h[17];
    hash16(color, len, h);
    emit(snprintf(line, sizeof line, "C %lu %08x %u %s %lld\n", (unsigned long)s, (unsigned)cmd, (unsigned)len, h,
                  (long long)esp_timer_get_time()));
    if (cap_dump) {
        dump(s, color, len);
    }
    return __real_esp_lcd_panel_io_tx_color(io, cmd, color, len);
}

esp_err_t __wrap_esp_lcd_panel_draw_bitmap(esp_lcd_panel_handle_t panel, int x0, int y0, int x1, int y1, const void *color)
{
    uint32_t s = ++seq;
    size_t len = (size_t)(x1 - x0) * (size_t)(y1 - y0) * 2;
    char h[17];
    hash16(color, len, h);
    emit(snprintf(line, sizeof line, "F %lu %d %d %d %d %u %s %lld\n", (unsigned long)s, x0, y0, x1 - 1, y1 - 1,
                  (unsigned)len, h, (long long)esp_timer_get_time()));
    return __real_esp_lcd_panel_draw_bitmap(panel, x0, y0, x1, y1, color);
}

static void spi_line(const char *api, const spi_transaction_t *t)
{
    const spi_transaction_ext_t *e = (const spi_transaction_ext_t *)t;
    char cb[8] = "-", ab[8] = "-";
    if (t->flags & SPI_TRANS_VARIABLE_CMD) {
        snprintf(cb, sizeof cb, "%u", e->command_bits);
    }
    if (t->flags & SPI_TRANS_VARIABLE_ADDR) {
        snprintf(ab, sizeof ab, "%u", e->address_bits);
    }
    emit(snprintf(line, sizeof line, "S %lu %s %04lx %s %x %s %llx %u\n", (unsigned long)seq, api, (unsigned long)t->flags,
                  cb, (unsigned)t->cmd, ab, (unsigned long long)t->addr, (unsigned)t->length));
}

esp_err_t __wrap_spi_device_queue_trans(spi_device_handle_t h, spi_transaction_t *t, TickType_t wait)
{
    spi_line("queue", t);
    return __real_spi_device_queue_trans(h, t, wait);
}

esp_err_t __wrap_spi_device_polling_transmit(spi_device_handle_t h, spi_transaction_t *t)
{
    spi_line("poll", t);
    return __real_spi_device_polling_transmit(h, t);
}

esp_err_t __wrap_spi_device_transmit(spi_device_handle_t h, spi_transaction_t *t)
{
    spi_line("transmit", t);
    return __real_spi_device_transmit(h, t);
}
