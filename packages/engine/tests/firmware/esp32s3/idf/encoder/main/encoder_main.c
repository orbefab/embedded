// The knob's two contacts, EC1_A on GPIO2 and EC1_B on GPIO1, each pulled up on the board and
// closed to ground for one detent. A falling-edge interrupt counts each contact's closings and
// takes the time, and a 3 ms esp_timer polls both pins and counts a press after two equal reads.
// Every edge prints "edge a=<n> b=<m> t=<us>", and every polled press "poll a=<n> b=<m>".
#include <inttypes.h>
#include <stdio.h>
#include "driver/gpio.h"
#include "esp_attr.h"
#include "esp_err.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "knobconsole.h"

#define PIN_A 2
#define PIN_B 1
#define POLL_US 3000

typedef struct {
    char kind;
    int a, b;
    int64_t t;
} event_t;

static const int pins[2] = {PIN_A, PIN_B};
static QueueHandle_t events;
static int edges[2];
static int presses[2];
static int state[2] = {1, 1};
static int seen[2] = {1, 1};

static void IRAM_ATTR on_edge(void *arg)
{
    int k = (int)arg;
    edges[k]++;
    event_t e = {'e', edges[0], edges[1], esp_timer_get_time()};
    BaseType_t woken = pdFALSE;
    xQueueSendFromISR(events, &e, &woken);
    portYIELD_FROM_ISR(woken);
}

static void poll(void *arg)
{
    for (int k = 0; k < 2; k++) {
        int level = gpio_get_level(pins[k]);
        if (level == seen[k] && level != state[k]) {
            state[k] = level;
            if (level == 0) {
                presses[k]++;
                event_t e = {'p', presses[0], presses[1], 0};
                xQueueSend(events, &e, 0);
            }
        }
        seen[k] = level;
    }
}

void app_main(void)
{
    knob_console_wait_go();
    knob_console_start();
    events = xQueueCreate(16, sizeof(event_t));

    const gpio_config_t io = {
        .pin_bit_mask = 1ULL << PIN_A | 1ULL << PIN_B,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_NEGEDGE,
    };
    ESP_ERROR_CHECK(gpio_config(&io));
    ESP_ERROR_CHECK(gpio_install_isr_service(0));
    ESP_ERROR_CHECK(gpio_isr_handler_add(PIN_A, on_edge, (void *)0));
    ESP_ERROR_CHECK(gpio_isr_handler_add(PIN_B, on_edge, (void *)1));

    const esp_timer_create_args_t timer_args = {.callback = poll, .name = "poll"};
    esp_timer_handle_t timer;
    ESP_ERROR_CHECK(esp_timer_create(&timer_args, &timer));
    ESP_ERROR_CHECK(esp_timer_start_periodic(timer, POLL_US));
    printf("encoder ready\n");

    for (;;) {
        event_t e;
        xQueueReceive(events, &e, portMAX_DELAY);
        if (e.kind == 'e') {
            printf("edge a=%d b=%d t=%" PRId64 "\n", e.a, e.b, e.t);
        } else {
            printf("poll a=%d b=%d\n", e.a, e.b);
        }
    }
}
