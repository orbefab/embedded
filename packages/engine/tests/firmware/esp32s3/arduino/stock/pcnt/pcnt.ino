#include <driver/pulse_cnt.h>

static const int PIN_A = 4;
static const int PIN_B = 5;
static const int HIGH_LIMIT = 8;
static const int LOW_LIMIT = -8;

static pcnt_unit_handle_t unit = nullptr;
static volatile int limit_hits = 0;
static volatile int last_limit = 0;

static bool IRAM_ATTR on_reach(pcnt_unit_handle_t, const pcnt_watch_event_data_t *edata, void *) {
  last_limit = edata->watch_point_value;
  limit_hits++;
  return false;
}

void setup() {
  Serial.begin(115200);

  pcnt_unit_config_t unit_cfg = {};
  unit_cfg.low_limit = LOW_LIMIT;
  unit_cfg.high_limit = HIGH_LIMIT;
  ESP_ERROR_CHECK(pcnt_new_unit(&unit_cfg, &unit));

  pcnt_glitch_filter_config_t filter_cfg = {};
  filter_cfg.max_glitch_ns = 5000;
  ESP_ERROR_CHECK(pcnt_unit_set_glitch_filter(unit, &filter_cfg));

  pcnt_chan_config_t ch_a_cfg = {};
  ch_a_cfg.edge_gpio_num = PIN_A;
  ch_a_cfg.level_gpio_num = PIN_B;
  pcnt_channel_handle_t ch_a = nullptr;
  ESP_ERROR_CHECK(pcnt_new_channel(unit, &ch_a_cfg, &ch_a));

  pcnt_chan_config_t ch_b_cfg = {};
  ch_b_cfg.edge_gpio_num = PIN_B;
  ch_b_cfg.level_gpio_num = PIN_A;
  pcnt_channel_handle_t ch_b = nullptr;
  ESP_ERROR_CHECK(pcnt_new_channel(unit, &ch_b_cfg, &ch_b));

  ESP_ERROR_CHECK(pcnt_channel_set_edge_action(
      ch_a, PCNT_CHANNEL_EDGE_ACTION_DECREASE, PCNT_CHANNEL_EDGE_ACTION_INCREASE));
  ESP_ERROR_CHECK(pcnt_channel_set_level_action(
      ch_a, PCNT_CHANNEL_LEVEL_ACTION_KEEP, PCNT_CHANNEL_LEVEL_ACTION_INVERSE));
  ESP_ERROR_CHECK(pcnt_channel_set_edge_action(
      ch_b, PCNT_CHANNEL_EDGE_ACTION_INCREASE, PCNT_CHANNEL_EDGE_ACTION_DECREASE));
  ESP_ERROR_CHECK(pcnt_channel_set_level_action(
      ch_b, PCNT_CHANNEL_LEVEL_ACTION_KEEP, PCNT_CHANNEL_LEVEL_ACTION_INVERSE));

  ESP_ERROR_CHECK(pcnt_unit_add_watch_point(unit, HIGH_LIMIT));
  ESP_ERROR_CHECK(pcnt_unit_add_watch_point(unit, LOW_LIMIT));

  pcnt_event_callbacks_t cbs = {};
  cbs.on_reach = on_reach;
  ESP_ERROR_CHECK(pcnt_unit_register_event_callbacks(unit, &cbs, nullptr));

  ESP_ERROR_CHECK(pcnt_unit_enable(unit));
  ESP_ERROR_CHECK(pcnt_unit_clear_count(unit));
  ESP_ERROR_CHECK(pcnt_unit_start(unit));

  Serial.println("ready");
}

static int printed_hits = 0;
static int last_count = 0;

void loop() {
  int hits = limit_hits;
  if (hits != printed_hits) {
    printed_hits = hits;
    Serial.print("limit ");
    Serial.println(last_limit);
  }
  int count = 0;
  ESP_ERROR_CHECK(pcnt_unit_get_count(unit, &count));
  if (count != last_count) {
    last_count = count;
    Serial.print("count ");
    Serial.println(count);
  }
  delay(1);
}
