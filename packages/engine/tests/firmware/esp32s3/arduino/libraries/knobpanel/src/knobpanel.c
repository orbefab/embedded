/*
 * QSPI panel IO and an ST77916 panel.
 * The panel half follows Espressif's esp_lcd_st77916 (Apache-2.0): the same opcode framing,
 * init loop and draw path, cut to the calls the knob sketch makes.
 * The build stages packages/engine/tests/firmware/esp32s3/idf/screen/main/vendor_init.inc beside this file.
 */
#include <stdlib.h>
#include "knobpanel.h"
#include "driver/gpio.h"
#include "driver/spi_master.h"
#include "esp_check.h"
#include "esp_lcd_panel_commands.h"
#include "esp_lcd_panel_interface.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define OPCODE_WRITE_CMD 0x02u
#define OPCODE_WRITE_COLOR 0x32u

static const char *TAG = "knobpanel";

const knob_init_cmd_t knob_vendor_init[] = {
#include "vendor_init.inc"
};
const size_t knob_vendor_init_len = sizeof(knob_vendor_init) / sizeof(knob_vendor_init[0]);

typedef struct {
    esp_lcd_panel_t base;
    esp_lcd_panel_io_handle_t io;
    const knob_init_cmd_t *cmds;
    size_t n;
    uint8_t madctl;
    uint8_t colmod;
} st77916_t;

esp_lcd_panel_io_handle_t knob_qspi_io(esp_lcd_panel_io_color_trans_done_cb_t done, void *ctx, size_t max_transfer)
{
    spi_bus_config_t bus = {
        .data0_io_num = KNOB_LCD_D0,
        .data1_io_num = KNOB_LCD_D1,
        .sclk_io_num = KNOB_LCD_SCK,
        .data2_io_num = KNOB_LCD_D2,
        .data3_io_num = KNOB_LCD_D3,
        .data4_io_num = -1,
        .data5_io_num = -1,
        .data6_io_num = -1,
        .data7_io_num = -1,
        .max_transfer_sz = (int)max_transfer,
    };
    ESP_ERROR_CHECK(spi_bus_initialize(SPI2_HOST, &bus, SPI_DMA_CH_AUTO));
    esp_lcd_panel_io_spi_config_t cfg = {
        .cs_gpio_num = KNOB_LCD_CS,
        .dc_gpio_num = -1,
        .spi_mode = 0,
        .pclk_hz = KNOB_LCD_PCLK_HZ,
        .trans_queue_depth = 10,
        .on_color_trans_done = done,
        .user_ctx = ctx,
        .lcd_cmd_bits = 32,
        .lcd_param_bits = 8,
        .flags = {.quad_mode = 1},
    };
    esp_lcd_panel_io_handle_t io = NULL;
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)SPI2_HOST, &cfg, &io));
    return io;
}

static esp_err_t tx_param(st77916_t *p, uint8_t cmd, const void *data, size_t len)
{
    return esp_lcd_panel_io_tx_param(p->io, (int)(OPCODE_WRITE_CMD << 24 | (uint32_t)cmd << 8), data, len);
}

static esp_err_t panel_reset(esp_lcd_panel_t *panel)
{
    gpio_set_level(KNOB_LCD_RST, 0);
    vTaskDelay(pdMS_TO_TICKS(10));
    gpio_set_level(KNOB_LCD_RST, 1);
    vTaskDelay(pdMS_TO_TICKS(120));
    return ESP_OK;
}

static esp_err_t panel_init(esp_lcd_panel_t *panel)
{
    st77916_t *p = __containerof(panel, st77916_t, base);
    ESP_RETURN_ON_ERROR(tx_param(p, LCD_CMD_MADCTL, (uint8_t[]){p->madctl}, 1), TAG, "madctl");
    ESP_RETURN_ON_ERROR(tx_param(p, LCD_CMD_COLMOD, (uint8_t[]){p->colmod}, 1), TAG, "colmod");
    for (size_t i = 0; i < p->n; i++) {
        ESP_RETURN_ON_ERROR(tx_param(p, p->cmds[i].cmd, p->cmds[i].data, p->cmds[i].len), TAG, "init");
        vTaskDelay(pdMS_TO_TICKS(p->cmds[i].delay_ms));
    }
    return ESP_OK;
}

static esp_err_t panel_draw_bitmap(esp_lcd_panel_t *panel, int x0, int y0, int x1, int y1, const void *color)
{
    st77916_t *p = __containerof(panel, st77916_t, base);
    ESP_RETURN_ON_ERROR(tx_param(p, LCD_CMD_CASET, (uint8_t[]){x0 >> 8, x0 & 0xff, (x1 - 1) >> 8, (x1 - 1) & 0xff}, 4), TAG, "caset");
    ESP_RETURN_ON_ERROR(tx_param(p, LCD_CMD_RASET, (uint8_t[]){y0 >> 8, y0 & 0xff, (y1 - 1) >> 8, (y1 - 1) & 0xff}, 4), TAG, "raset");
    size_t len = (size_t)(x1 - x0) * (size_t)(y1 - y0) * 2;
    return esp_lcd_panel_io_tx_color(p->io, (int)(OPCODE_WRITE_COLOR << 24 | (uint32_t)LCD_CMD_RAMWR << 8), color, len);
}

static esp_err_t panel_invert(esp_lcd_panel_t *panel, bool invert)
{
    return tx_param(__containerof(panel, st77916_t, base), invert ? LCD_CMD_INVON : LCD_CMD_INVOFF, NULL, 0);
}

static esp_err_t panel_on_off(esp_lcd_panel_t *panel, bool on)
{
    return tx_param(__containerof(panel, st77916_t, base), on ? LCD_CMD_DISPON : LCD_CMD_DISPOFF, NULL, 0);
}

static esp_err_t panel_del(esp_lcd_panel_t *panel)
{
    free(__containerof(panel, st77916_t, base));
    return ESP_OK;
}

esp_lcd_panel_handle_t knob_st77916(esp_lcd_panel_io_handle_t io, const knob_init_cmd_t *cmds, size_t n)
{
    gpio_config_t rst = {.mode = GPIO_MODE_OUTPUT, .pin_bit_mask = 1ULL << KNOB_LCD_RST};
    ESP_ERROR_CHECK(gpio_config(&rst));
    st77916_t *p = calloc(1, sizeof(*p));
    assert(p);
    p->io = io;
    p->cmds = cmds;
    p->n = n;
    p->madctl = 0x00;
    p->colmod = 0x55;
    p->base.reset = panel_reset;
    p->base.init = panel_init;
    p->base.del = panel_del;
    p->base.draw_bitmap = panel_draw_bitmap;
    p->base.invert_color = panel_invert;
    p->base.disp_on_off = panel_on_off;
    return &p->base;
}
