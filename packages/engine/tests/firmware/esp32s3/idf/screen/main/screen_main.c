// An IDF LVGL screen on an ST77916 panel over QSPI through esp_lcd_st77916 2.0.2, with the panel
// maker's init sequence. It draws an arc and a label, then ten fixed updates, and wrap.c logs
// every transfer. Nothing on screen depends on time.
#include <stdio.h>
#include "driver/ledc.h"
#include "driver/spi_master.h"
#include "esp_chip_info.h"
#include "esp_err.h"
#include "esp_heap_caps.h"
#include "esp_idf_version.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_st77916.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "knobconsole.h"
#include "lvgl.h"

#define LCD_CS 12
#define LCD_SCK 11
#define LCD_D0 13
#define LCD_D1 14
#define LCD_D2 15
#define LCD_D3 16
#define LCD_RST 17
#define LCD_BLK 21
#define LCD_RES 360
#define LV_ROWS 72
#define UPDATES 10

extern volatile int cap_dump;

void cap_write(const char *s, size_t n)
{
    fwrite(s, 1, n, stdout);
}

static const st77916_lcd_init_cmd_t vendor_init[] = {
#include "vendor_init.inc"
};

static esp_lcd_panel_handle_t panel;
static lv_display_t *disp;
static lv_obj_t *arc;
static lv_obj_t *value_label;
static volatile uint32_t flushes_sent, flushes_done;

static bool color_done(esp_lcd_panel_io_handle_t io, esp_lcd_panel_io_event_data_t *e, void *ctx)
{
    flushes_done++;
    lv_display_flush_ready((lv_display_t *)ctx);
    return false;
}

static void flush(lv_display_t *d, const lv_area_t *a, uint8_t *px)
{
    lv_draw_sw_rgb565_swap(px, lv_area_get_size(a));
    flushes_sent++;
    esp_lcd_panel_draw_bitmap(panel, a->x1, a->y1, a->x2 + 1, a->y2 + 1, px);
}

static uint32_t tick_ms(void)
{
    return (uint32_t)(esp_timer_get_time() / 1000);
}

static void backlight(uint32_t duty)
{
    ESP_ERROR_CHECK(ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, duty));
    ESP_ERROR_CHECK(ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0));
}

static void build_screen(void)
{
    lv_obj_t *scr = lv_screen_active();
    lv_obj_set_style_bg_color(scr, lv_color_hex(0x101830), 0);
    lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);

    arc = lv_arc_create(scr);
    lv_obj_set_size(arc, 300, 300);
    lv_obj_center(arc);
    lv_arc_set_range(arc, 0, 100);
    lv_arc_set_value(arc, 0);
    lv_obj_set_style_arc_width(arc, 18, LV_PART_MAIN);
    lv_obj_set_style_arc_width(arc, 18, LV_PART_INDICATOR);
    lv_obj_set_style_arc_color(arc, lv_color_hex(0x00c0ff), LV_PART_INDICATOR);
    lv_obj_remove_style(arc, NULL, LV_PART_KNOB);

    value_label = lv_label_create(scr);
    lv_obj_set_style_text_font(value_label, &lv_font_montserrat_48, 0);
    lv_obj_set_style_text_color(value_label, lv_color_hex(0xffffff), 0);
    lv_label_set_text(value_label, "0");
    lv_obj_center(value_label);
}

static void frame(int k)
{
    printf("LCD FRAME %d BEGIN\n", k);
    lv_refr_now(disp);
    while (flushes_done != flushes_sent) {
    }
    printf("LCD FRAME %d END\n", k);
    vTaskDelay(1);
}

void app_main(void)
{
    knob_console_wait_go();
    knob_console_start();

    esp_chip_info_t chip;
    esp_chip_info(&chip);
    printf("LCD CAPTURE START\n");
    printf("LCD INFO reset_reason %d\n", (int)esp_reset_reason());
    printf("LCD INFO chip model %d revision %d cores %d\n", (int)chip.model, chip.revision, chip.cores);
    printf("LCD INFO idf %s lvgl %d.%d.%d\n", esp_get_idf_version(), LVGL_VERSION_MAJOR, LVGL_VERSION_MINOR, LVGL_VERSION_PATCH);

    ledc_timer_config_t timer = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .duty_resolution = LEDC_TIMER_12_BIT,
        .timer_num = LEDC_TIMER_0,
        .freq_hz = 5000,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    ESP_ERROR_CHECK(ledc_timer_config(&timer));
    ledc_channel_config_t channel = {
        .gpio_num = LCD_BLK,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = LEDC_CHANNEL_0,
        .timer_sel = LEDC_TIMER_0,
        .duty = 0,
    };
    ESP_ERROR_CHECK(ledc_channel_config(&channel));

    cap_dump = 1;
    printf("LCD PANEL BEGIN\n");
    const spi_bus_config_t bus = ST77916_PANEL_BUS_QSPI_CONFIG(LCD_SCK, LCD_D0, LCD_D1, LCD_D2, LCD_D3, LCD_RES * LV_ROWS * 2);
    ESP_ERROR_CHECK(spi_bus_initialize(SPI2_HOST, &bus, SPI_DMA_CH_AUTO));
    const esp_lcd_panel_io_spi_config_t io_cfg = ST77916_PANEL_IO_QSPI_CONFIG(LCD_CS, NULL, NULL);
    esp_lcd_panel_io_handle_t io = NULL;
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)SPI2_HOST, &io_cfg, &io));
    st77916_vendor_config_t vendor = {
        .init_cmds = vendor_init,
        .init_cmds_size = sizeof(vendor_init) / sizeof(vendor_init[0]),
        .flags = {.use_qspi_interface = 1},
    };
    const esp_lcd_panel_dev_config_t panel_cfg = {
        .reset_gpio_num = LCD_RST,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
        .vendor_config = &vendor,
    };
    ESP_ERROR_CHECK(esp_lcd_new_panel_st77916(io, &panel_cfg, &panel));
    ESP_ERROR_CHECK(esp_lcd_panel_reset(panel));
    ESP_ERROR_CHECK(esp_lcd_panel_init(panel));
    ESP_ERROR_CHECK(esp_lcd_panel_invert_color(panel, true));
    ESP_ERROR_CHECK(esp_lcd_panel_disp_on_off(panel, true));
    printf("LCD PANEL END\n");
    backlight(4095);

    lv_init();
    lv_tick_set_cb(tick_ms);
    disp = lv_display_create(LCD_RES, LCD_RES);
    size_t buf_bytes = LCD_RES * LV_ROWS * 2;
    void *buf = heap_caps_malloc(buf_bytes, MALLOC_CAP_DMA | MALLOC_CAP_INTERNAL);
    assert(buf);
    lv_display_set_buffers(disp, buf, NULL, buf_bytes, LV_DISPLAY_RENDER_MODE_PARTIAL);
    lv_display_set_flush_cb(disp, flush);
    const esp_lcd_panel_io_callbacks_t cbs = {.on_color_trans_done = color_done};
    ESP_ERROR_CHECK(esp_lcd_panel_io_register_event_callbacks(io, &cbs, disp));

    build_screen();
    frame(0);
    for (int k = 1; k <= UPDATES; k++) {
        lv_arc_set_value(arc, k * 10);
        lv_label_set_text_fmt(value_label, "%d", k * 10);
        frame(k);
    }
    cap_dump = 0;
    printf("LCD DONE\n");

    for (;;) {
        lv_timer_handler();
        vTaskDelay(pdMS_TO_TICKS(10));
    }
}
