// Twelve LEDs on GPIO0. Each table entry is red, green, blue. A 1 is 8 ticks high and 4
// low, and a 0 is 3 high and 9 low. A tick is 100 ns. The app waits for a console g, waits
// 20 ms, sends the frame, and prints ring ready, frame n and ring done.
#include <stdio.h>
#include "driver/rmt_tx.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "knobconsole.h"

#define LEDS 12
#define FRAMES 3

static const uint8_t frames[FRAMES][LEDS][3] = {
    {{24, 0, 0},
     {24, 24, 24},
     {24, 24, 24},
     {24, 24, 24},
     {24, 24, 24},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0}},
    {{128, 0, 0},
     {0, 128, 0},
     {0, 0, 128},
     {1, 2, 4},
     {85, 0, 0},
     {0, 42, 0},
     {0, 0, 21},
     {16, 32, 48},
     {15, 0, 15},
     {0, 15, 15},
     {60, 24, 0},
     {1, 1, 1}},
    {{0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 0},
     {0, 0, 48}},
};

static rmt_symbol_word_t bits[LEDS * 24];

static void encode(const uint8_t (*rgb)[3])
{
    size_t n = 0;
    for (int i = 0; i < LEDS; i++) {
        const uint8_t grb[3] = {rgb[i][1], rgb[i][0], rgb[i][2]};
        for (int channel = 0; channel < 3; channel++) {
            for (int bit = 7; bit >= 0; bit--) {
                const int one = grb[channel] >> bit & 1;
                bits[n++] = (rmt_symbol_word_t){
                    .level0 = 1,
                    .duration0 = one ? 8 : 3,
                    .level1 = 0,
                    .duration1 = one ? 4 : 9,
                };
            }
        }
    }
}

void app_main(void)
{
    knob_console_wait_go();
    const rmt_tx_channel_config_t config = {
        .gpio_num = 0,
        .clk_src = RMT_CLK_SRC_DEFAULT,
        .resolution_hz = 10000000,
        .mem_block_symbols = 48,
        .trans_queue_depth = 10,
    };
    rmt_channel_handle_t channel;
    ESP_ERROR_CHECK(rmt_new_tx_channel(&config, &channel));
    const rmt_copy_encoder_config_t copy = {};
    rmt_encoder_handle_t encoder;
    ESP_ERROR_CHECK(rmt_new_copy_encoder(&copy, &encoder));
    ESP_ERROR_CHECK(rmt_enable(channel));
    printf("ring ready\n");
    const rmt_transmit_config_t once = {.loop_count = 0};
    for (int frame = 0; frame < FRAMES; frame++) {
        if (frame > 0) {
            knob_console_wait_go();
        }
        vTaskDelay(pdMS_TO_TICKS(20));
        encode(frames[frame]);
        ESP_ERROR_CHECK(rmt_transmit(channel, encoder, bits, sizeof bits, &once));
        ESP_ERROR_CHECK(rmt_tx_wait_all_done(channel, -1));
        printf("frame %d\n", frame);
    }
    knob_console_start();
    printf("ring done\n");
}
