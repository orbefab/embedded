// Waits for 'g', then times TIMG0 T0 alarms at two APB dividers and reads the RNG.
//
// Each alarm is 1000 counts. Divider 2 is a 40 MHz count and divider 80 is 1 MHz, both from
// the 80 MHz APB clock. The line is the cycles from the CCOUNT sample taken immediately before
// gptimer_start to the first instruction of the alarm callback. Eight one-shots, then eight
// auto-reload callbacks, at each divider. Then an alarm armed at the current count, an alarm
// armed behind the count, and an auto-reload whose load is above the alarm. The RNG lines say
// only whether the word was zero.
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "driver/gptimer.h"
#include "driver/usb_serial_jtag.h"
#include "esp_cpu.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "soc/timer_group_reg.h"
#include "soc/wdev_reg.h"

#define ALARMS 8
#define ALARM_COUNT 1000

static volatile uint32_t hits;
static volatile uint32_t stamps[ALARMS];
// The above-alarm reload re-enables itself inside the driver's interrupt. Clear INT_ENA once
// this many callbacks have run, so the level interrupt stops. Zero leaves every run alone.
static volatile uint32_t stop_after;

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

static void print_line(const char *fmt, ...)
{
    char line[64];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    write_all(line);
}

// Level 3 is the timer. Level 2 blocks the tick and leaves the timer able to preempt.
static uint32_t mask_below_timer(void)
{
    uint32_t old;
    asm volatile("rsil %0, 2" : "=a"(old));
    return old;
}

static void restore_ps(uint32_t old)
{
    asm volatile("wsr %0, PS; rsync" :: "a"(old));
}

static bool IRAM_ATTR on_alarm(gptimer_handle_t timer, const gptimer_alarm_event_data_t *edata, void *user_ctx)
{
    uint32_t now = esp_cpu_get_cycle_count();
    uint32_t n = hits;
    (void)timer;
    (void)edata;
    (void)user_ctx;
    if (n < ALARMS) {
        stamps[n] = now;
        hits = n + 1;
    }
    if (stop_after != 0 && n + 1 >= stop_after) {
        REG_WRITE(TIMG_INT_ENA_TIMERS_REG(0), 0);
    }
    return false;
}

static uint32_t t0_divider(void)
{
    uint32_t field = (REG_READ(TIMG_T0CONFIG_REG(0)) & TIMG_T0_DIVIDER_M) >> TIMG_T0_DIVIDER_S;
    return field == 0 ? 65536 : field;
}

// Samples CCOUNT, starts the counter, and returns that sample. The tick cannot run between them.
static uint32_t sample_and_start(gptimer_handle_t timer)
{
    uint32_t old = mask_below_timer();
    uint32_t start = esp_cpu_get_cycle_count();
    esp_err_t err = gptimer_start(timer);
    restore_ps(old);
    ESP_ERROR_CHECK(err);
    return start;
}

// `waiti 2` sleeps until an interrupt above level 2. The alarm is level 3, and the tick is
// level 1, so the tick does not wake the wait. Waking from `waiti` enters the callback on the
// alarm's cycle, and a spin would enter it one cycle to either side of the instruction it was in.
static void wait_hits(uint32_t want)
{
    uint32_t old = mask_below_timer();
    while (hits < want) {
        asm volatile("waiti 2" ::: "memory");
    }
    restore_ps(old);
}

static void run_divider(uint32_t resolution_hz, uint32_t divider)
{
    gptimer_handle_t timer = NULL;
    gptimer_config_t config = {
        .clk_src = GPTIMER_CLK_SRC_APB,
        .direction = GPTIMER_COUNT_UP,
        .resolution_hz = resolution_hz,
        .intr_priority = 3,
    };
    ESP_ERROR_CHECK(gptimer_new_timer(&config, &timer));
    uint32_t got = t0_divider();
    if (got != divider || (REG_READ(TIMG_T0CONFIG_REG(0)) & TIMG_T0_USE_XTAL) != 0) {
        print_line("div %u got %u\n", (unsigned)divider, (unsigned)got);
        return;
    }

    gptimer_event_callbacks_t callbacks = {.on_alarm = on_alarm};
    ESP_ERROR_CHECK(gptimer_register_event_callbacks(timer, &callbacks, NULL));
    gptimer_alarm_config_t alarm = {
        .alarm_count = ALARM_COUNT,
        .reload_count = 0,
    };
    ESP_ERROR_CHECK(gptimer_set_alarm_action(timer, &alarm));
    ESP_ERROR_CHECK(gptimer_enable(timer));

    for (int k = 0; k < ALARMS; k++) {
        hits = 0;
        ESP_ERROR_CHECK(gptimer_set_raw_count(timer, 0));
        uint32_t start = sample_and_start(timer);
        wait_hits(1);
        uint32_t cycles = stamps[0] - start;
        ESP_ERROR_CHECK(gptimer_stop(timer));
        print_line("div %u shot %d %u\n", (unsigned)divider, k, cycles);
    }

    alarm.flags.auto_reload_on_alarm = true;
    ESP_ERROR_CHECK(gptimer_set_alarm_action(timer, &alarm));
    hits = 0;
    ESP_ERROR_CHECK(gptimer_set_raw_count(timer, 0));
    uint32_t start = sample_and_start(timer);
    wait_hits(ALARMS);
    ESP_ERROR_CHECK(gptimer_stop(timer));
    for (int k = 0; k < ALARMS; k++) {
        print_line("div %u reload %d %u\n", (unsigned)divider, k, stamps[k] - start);
    }

    ESP_ERROR_CHECK(gptimer_disable(timer));
    ESP_ERROR_CHECK(gptimer_del_timer(timer));
}

// One one-shot at divider 2. `equal` arms the alarm at the loaded count. `behind` arms it
// below that count. Both fire at the store that starts the counter.
static void run_level(const char *name, uint64_t count, uint64_t alarm_at)
{
    gptimer_handle_t timer = NULL;
    gptimer_config_t config = {
        .clk_src = GPTIMER_CLK_SRC_APB,
        .direction = GPTIMER_COUNT_UP,
        .resolution_hz = 40 * 1000 * 1000,
        .intr_priority = 3,
    };
    ESP_ERROR_CHECK(gptimer_new_timer(&config, &timer));
    gptimer_event_callbacks_t callbacks = {.on_alarm = on_alarm};
    ESP_ERROR_CHECK(gptimer_register_event_callbacks(timer, &callbacks, NULL));
    gptimer_alarm_config_t alarm = {
        .alarm_count = alarm_at,
        .reload_count = 0,
    };
    ESP_ERROR_CHECK(gptimer_set_alarm_action(timer, &alarm));
    ESP_ERROR_CHECK(gptimer_enable(timer));
    hits = 0;
    ESP_ERROR_CHECK(gptimer_set_raw_count(timer, count));
    uint32_t start = sample_and_start(timer);
    wait_hits(1);
    ESP_ERROR_CHECK(gptimer_stop(timer));
    print_line("%s %u\n", name, stamps[0] - start);
    ESP_ERROR_CHECK(gptimer_disable(timer));
    ESP_ERROR_CHECK(gptimer_del_timer(timer));
}

// Divider 80, alarm 1000, reload 5000. The first callback is the count reaching the alarm.
// The driver's interrupt then re-enables the alarm with the reload already above it, so the
// second callback is immediate. INT_ENA is cleared from the second callback.
static void run_above(void)
{
    gptimer_handle_t timer = NULL;
    gptimer_config_t config = {
        .clk_src = GPTIMER_CLK_SRC_APB,
        .direction = GPTIMER_COUNT_UP,
        .resolution_hz = 1 * 1000 * 1000,
        .intr_priority = 3,
    };
    ESP_ERROR_CHECK(gptimer_new_timer(&config, &timer));
    gptimer_event_callbacks_t callbacks = {.on_alarm = on_alarm};
    ESP_ERROR_CHECK(gptimer_register_event_callbacks(timer, &callbacks, NULL));
    gptimer_alarm_config_t alarm = {
        .alarm_count = ALARM_COUNT,
        .reload_count = 5000,
    };
    alarm.flags.auto_reload_on_alarm = true;
    ESP_ERROR_CHECK(gptimer_set_alarm_action(timer, &alarm));
    ESP_ERROR_CHECK(gptimer_enable(timer));
    stop_after = 2;
    hits = 0;
    ESP_ERROR_CHECK(gptimer_set_raw_count(timer, 0));
    uint32_t start = sample_and_start(timer);
    wait_hits(2);
    stop_after = 0;
    ESP_ERROR_CHECK(gptimer_stop(timer));
    print_line("above %u %u\n", stamps[0] - start, stamps[1] - stamps[0]);
    ESP_ERROR_CHECK(gptimer_disable(timer));
    ESP_ERROR_CHECK(gptimer_del_timer(timer));
}

void app_main(void)
{
    usb_serial_jtag_driver_config_t usb = USB_SERIAL_JTAG_DRIVER_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&usb));
    for (;;) {
        uint8_t byte = 0;
        if (usb_serial_jtag_read_bytes(&byte, 1, portMAX_DELAY) == 1 && byte == 'g') {
            break;
        }
    }

    run_divider(40 * 1000 * 1000, 2);
    run_divider(1 * 1000 * 1000, 80);
    run_level("equal", 1000, 1000);
    run_level("behind", 2000, 1000);
    run_above();

    for (int k = 0; k < ALARMS; k++) {
        uint32_t word = REG_READ(WDEV_RND_REG);
        print_line("rng %d %s\n", k, word == 0 ? "z" : "nz");
    }
    write_all("TIMERS DONE\n");
}
