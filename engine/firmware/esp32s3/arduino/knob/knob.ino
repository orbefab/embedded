#include <climits>
#include <Wire.h>
#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>
#include <freertos/task.h>
#include <lvgl.h>
#include <knobpanel.h>
#include <knobconsole.h>
#include "esp_chip_info.h"
#include "esp_idf_version.h"
#include "esp_system.h"

#define TOUCH_SDA 9
#define TOUCH_SCL 10
#define TOUCH_INT 7
#define TOUCH_RST 8
#define TOUCH_ADDR 0x15
#define ENC_A 2
#define ENC_B 1
#define LV_ROWS 72
#define UPDATES 10

extern "C" volatile int cap_dump;

extern "C" void cap_write(const char *s, size_t n)
{
  Serial.write((const uint8_t *)s, n);
}

struct Count {
  int a;
  int b;
};

static esp_lcd_panel_handle_t panel;
static lv_display_t *disp;
static lv_obj_t *arc;
static lv_obj_t *value_label;
static int touch_last;
static volatile uint32_t flushes_sent, flushes_done;
static QueueHandle_t counts;
static bool finished;

static const int dot_x[3] = {180, 93, 267};
static const int dot_y[3] = {80, 230, 230};

static bool color_done(esp_lcd_panel_io_handle_t, esp_lcd_panel_io_event_data_t *, void *ctx)
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

static int nearest_dot(int x, int y)
{
  int best = 0;
  long best_d = LONG_MAX;
  for (int i = 0; i < 3; i++) {
    long dx = x - dot_x[i];
    long dy = y - dot_y[i];
    long d = dx * dx + dy * dy;
    if (d < best_d) {
      best_d = d;
      best = i;
    }
  }
  return best + 1;
}

static void touch_read(lv_indev_t *, lv_indev_data_t *data)
{
  uint8_t buf[5] = {0};
  Wire.beginTransmission(TOUCH_ADDR);
  Wire.write(0x02);
  bool ok = Wire.endTransmission(false) == 0 && Wire.requestFrom(TOUCH_ADDR, 5) == 5;
  if (ok) {
    for (int i = 0; i < 5; i++) buf[i] = Wire.read();
  }
  int fingers = ok ? (buf[0] & 0x0f) : 0;
  int x = (buf[1] & 0x0f) << 8 | buf[2];
  int y = (buf[3] & 0x0f) << 8 | buf[4];
  if (fingers > 0) {
    data->point.x = x;
    data->point.y = y;
    data->state = LV_INDEV_STATE_PRESSED;
    if (touch_last == 0) Serial.printf("T down %d\n", nearest_dot(x, y));
  } else {
    data->state = LV_INDEV_STATE_RELEASED;
    if (touch_last > 0) Serial.println("T up");
  }
  touch_last = fingers;
}

static void contact_task(void *)
{
  const int pins[2] = {ENC_A, ENC_B};
  int state[2] = {1, 1};
  int seen[2] = {1, 1};
  int presses[2] = {0, 0};
  for (;;) {
    for (int k = 0; k < 2; k++) {
      int level = digitalRead(pins[k]);
      if (level == seen[k] && level != state[k]) {
        state[k] = level;
        if (level == 0) {
          presses[k]++;
          Count count = {presses[0], presses[1]};
          xQueueSend(counts, &count, 0);
        }
      }
      seen[k] = level;
    }
    vTaskDelay(1);
  }
}

static void build_screen()
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
  lv_obj_set_style_arc_color(arc, lv_color_hex(0xff8000), LV_PART_INDICATOR);
  lv_obj_remove_style(arc, NULL, LV_PART_KNOB);
  lv_obj_remove_flag(arc, LV_OBJ_FLAG_CLICKABLE);

  value_label = lv_label_create(scr);
  lv_obj_set_style_text_font(value_label, &lv_font_montserrat_48, 0);
  lv_obj_set_style_text_color(value_label, lv_color_hex(0xffffff), 0);
  lv_label_set_text(value_label, "0");
  lv_obj_center(value_label);

  lv_obj_t *title = lv_label_create(scr);
  lv_obj_set_style_text_font(title, &lv_font_montserrat_28, 0);
  lv_obj_set_style_text_color(title, lv_color_hex(0x80c0ff), 0);
  lv_label_set_text(title, "KNOB");
  lv_obj_align(title, LV_ALIGN_TOP_MID, 0, 90);

  const uint32_t colors[3] = {0xff0000, 0x00ff00, 0x0000ff};
  for (int i = 0; i < 3; i++) {
    lv_obj_t *dot = lv_obj_create(scr);
    lv_obj_remove_style_all(dot);
    lv_obj_set_size(dot, 16, 16);
    lv_obj_set_pos(dot, dot_x[i] - 8, dot_y[i] - 8);
    lv_obj_set_style_radius(dot, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_bg_color(dot, lv_color_hex(colors[i]), 0);
    lv_obj_set_style_bg_opa(dot, LV_OPA_COVER, 0);
    lv_obj_remove_flag(dot, LV_OBJ_FLAG_CLICKABLE);
  }
}

static void frame(int k)
{
  Serial.printf("KNOB FRAME %d BEGIN\n", k);
  lv_refr_now(disp);
  while (flushes_done != flushes_sent) {
  }
  Serial.printf("KNOB FRAME %d END\n", k);
}

void setup()
{
  knob_console_begin();
  pinMode(ENC_A, INPUT_PULLUP);
  pinMode(ENC_B, INPUT_PULLUP);
  counts = xQueueCreate(32, sizeof(Count));
  xTaskCreatePinnedToCore(contact_task, "contacts", 4096, nullptr, 5, nullptr, 0);
  knob_console_wait_go();

  esp_chip_info_t chip;
  esp_chip_info(&chip);
  Serial.println("KNOB START");
  Serial.printf("KNOB INFO reset_reason %d\n", (int)esp_reset_reason());
  Serial.printf("KNOB INFO chip model %d revision %d cores %d\n", (int)chip.model, chip.revision, chip.cores);
  Serial.printf("KNOB INFO idf %s arduino %d.%d.%d\n", esp_get_idf_version(), ESP_ARDUINO_VERSION_MAJOR, ESP_ARDUINO_VERSION_MINOR,
                ESP_ARDUINO_VERSION_PATCH);
  Serial.printf("KNOB INFO psram_found %d psram_size %lu flash_size %lu\n", (int)psramFound(), (unsigned long)ESP.getPsramSize(),
                (unsigned long)ESP.getFlashChipSize());
  Serial.printf("KNOB INFO lvgl %d.%d.%d\n", LVGL_VERSION_MAJOR, LVGL_VERSION_MINOR, LVGL_VERSION_PATCH);

  ledcAttach(KNOB_LCD_BLK, 5000, 12);
  ledcWrite(KNOB_LCD_BLK, 0);

  pinMode(TOUCH_INT, INPUT_PULLUP);
  pinMode(TOUCH_RST, OUTPUT);
  digitalWrite(TOUCH_RST, LOW);
  delay(10);
  digitalWrite(TOUCH_RST, HIGH);
  delay(50);
  Wire.begin(TOUCH_SDA, TOUCH_SCL, 400000);

  cap_dump = 1;
  Serial.println("KNOB PANEL BEGIN");
  esp_lcd_panel_io_handle_t io = knob_qspi_io(nullptr, nullptr, KNOB_LCD_RES * LV_ROWS * 2);
  panel = knob_st77916(io, knob_vendor_init, knob_vendor_init_len);
  ESP_ERROR_CHECK(esp_lcd_panel_reset(panel));
  ESP_ERROR_CHECK(esp_lcd_panel_init(panel));
  ESP_ERROR_CHECK(esp_lcd_panel_invert_color(panel, true));
  ESP_ERROR_CHECK(esp_lcd_panel_disp_on_off(panel, true));
  Serial.println("KNOB PANEL END");
  ledcWrite(KNOB_LCD_BLK, 4095);

  lv_init();
  lv_tick_set_cb([]() -> uint32_t { return millis(); });
  disp = lv_display_create(KNOB_LCD_RES, KNOB_LCD_RES);
  size_t buf_bytes = KNOB_LCD_RES * LV_ROWS * 2;
  void *buf = heap_caps_malloc(buf_bytes, MALLOC_CAP_DMA | MALLOC_CAP_INTERNAL);
  lv_display_set_buffers(disp, buf, nullptr, buf_bytes, LV_DISPLAY_RENDER_MODE_PARTIAL);
  lv_display_set_flush_cb(disp, flush);
  esp_lcd_panel_io_callbacks_t cbs = {.on_color_trans_done = color_done};
  ESP_ERROR_CHECK(esp_lcd_panel_io_register_event_callbacks(io, &cbs, disp));
  lv_indev_t *touch = lv_indev_create();
  lv_indev_set_type(touch, LV_INDEV_TYPE_POINTER);
  lv_indev_set_read_cb(touch, touch_read);

  build_screen();
  frame(0);
  for (int k = 1; k <= UPDATES; k++) {
    lv_arc_set_value(arc, k * 10);
    lv_label_set_text_fmt(value_label, "%d", k * 10);
    frame(k);
  }
  cap_dump = 0;
  Serial.println("KNOB READY");
}

void loop()
{
  lv_timer_handler();
  if (!finished) {
    Count count;
    while (xQueueReceive(counts, &count, 0) == pdTRUE) {
      Serial.printf("K a=%d b=%d\n", count.a, count.b);
    }
  }
  int ch = Serial.read();
  if (ch == 'x' && !finished) {
    Serial.println("KNOB DONE");
    Serial.flush();
    finished = true;
  }
  knob_console_command(ch);
  delay(1);
}
