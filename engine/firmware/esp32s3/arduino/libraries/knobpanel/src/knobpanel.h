#pragma once
#include <stddef.h>
#include <stdint.h>
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"

#ifdef __cplusplus
extern "C" {
#endif

#define KNOB_LCD_CS 12
#define KNOB_LCD_SCK 11
#define KNOB_LCD_D0 13
#define KNOB_LCD_D1 14
#define KNOB_LCD_D2 15
#define KNOB_LCD_D3 16
#define KNOB_LCD_RST 17
#define KNOB_LCD_BLK 21
#define KNOB_LCD_PCLK_HZ (40 * 1000 * 1000)
#define KNOB_LCD_RES 360

typedef struct {
    uint8_t cmd;
    const uint8_t *data;
    uint8_t len;
    uint16_t delay_ms;
} knob_init_cmd_t;

extern const knob_init_cmd_t knob_vendor_init[];
extern const size_t knob_vendor_init_len;

esp_lcd_panel_io_handle_t knob_qspi_io(esp_lcd_panel_io_color_trans_done_cb_t done, void *ctx, size_t max_transfer);
esp_lcd_panel_handle_t knob_st77916(esp_lcd_panel_io_handle_t io, const knob_init_cmd_t *cmds, size_t n);

#ifdef __cplusplus
}
#endif
