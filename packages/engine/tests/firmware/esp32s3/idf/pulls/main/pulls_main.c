// GPIO44's internal pulls, on a pad with nothing attached. After 'g' on the console it reads
// GPIO_IN1 with the pull-up only, the pull-down only, both and neither, 1 ms after each, and
// prints "static up|down|both|none <level>". It then times the poll loop over a pad it drives,
// "poll <polls> <cycles>". Each of 16 repetitions k then waits k/16 of a poll after each
// release before its first poll, so the reps' first polls spread over one poll period. A rep
// releases a pad driven low under the pull-down, which reads 0 at the first poll, as its base;
// drives the pad high, sets the pull-down only, releases it and polls GPIO_IN1 against CCOUNT
// with interrupts masked until it reads 0; and does the same from low with the pull-up only
// until it reads 1. It prints "rep <k> base <cycles> fall <cycles> rise <cycles>", each from the
// release to the poll that read its level, 0 if none did. The mean of fall less base over the
// reps is the fall's time less a fixed offset, to within the reps' spread, and so is the rise's.
// The pad's capacitance cancels in their ratio, which is the pull-down's resistance over the
// pull-up's while the pad switches at half the supply on both edges. The app ends with
// "PULLS DONE".
#include <stdint.h>
#include <stdio.h>
#include "driver/gpio.h"
#include "esp_attr.h"
#include "esp_err.h"
#include "esp_rom_sys.h"
#include "knobconsole.h"
#include "soc/gpio_reg.h"
#include "soc/io_mux_reg.h"

#define PAD 44
#define PAD_BIT (1u << (PAD - 32))
#define REPS 16
#define LIMIT 100000u
#define POLLS 1000u

static inline uint32_t IRAM_ATTR ccount(void)
{
    uint32_t cycles;
    __asm__ volatile("rsr.ccount %0" : "=a"(cycles));
    return cycles;
}

static inline uint32_t IRAM_ATTR mask_interrupts(void)
{
    uint32_t ps;
    __asm__ volatile("rsil %0, 15" : "=a"(ps));
    return ps;
}

static inline void IRAM_ATTR restore_interrupts(uint32_t ps)
{
    __asm__ volatile("wsr.ps %0\n\trsync" : : "a"(ps) : "memory");
}

// Polls GPIO_IN1 until the pad reads `level` or `limit` polls have gone by, and returns the
// polls it took.
static uint32_t IRAM_ATTR poll(uint32_t level, uint32_t limit)
{
    const uint32_t want = level ? PAD_BIT : 0;
    uint32_t polls = 0;
    while ((REG_READ(GPIO_IN1_REG) & PAD_BIT) != want && polls < limit) {
        polls++;
    }
    return polls;
}

// Drives the pad at `from` with `mux` as its IO_MUX word, releases it, waits `wait` cycles,
// and returns the cycles from the release to the poll that reads `level`, or 0 if none does.
static uint32_t IRAM_ATTR settle(uint32_t from, uint32_t mux, uint32_t level, uint32_t wait)
{
    REG_WRITE(from ? GPIO_OUT1_W1TS_REG : GPIO_OUT1_W1TC_REG, PAD_BIT);
    REG_WRITE(GPIO_ENABLE1_W1TS_REG, PAD_BIT);
    REG_WRITE(IO_MUX_GPIO44_REG, mux);
    esp_rom_delay_us(20);
    const uint32_t ps = mask_interrupts();
    REG_WRITE(GPIO_ENABLE1_W1TC_REG, PAD_BIT);
    const uint32_t start = ccount();
    while (ccount() - start < wait) {
    }
    const uint32_t polls = poll(level, LIMIT);
    const uint32_t end = ccount();
    restore_interrupts(ps);
    return polls < LIMIT ? end - start : 0;
}

// The pad's level 1 ms after it takes `mux` as its IO_MUX word, with its output off.
static uint32_t level_with(uint32_t mux)
{
    REG_WRITE(IO_MUX_GPIO44_REG, mux);
    esp_rom_delay_us(1000);
    return (REG_READ(GPIO_IN1_REG) & PAD_BIT) != 0;
}

// The cycles of `POLLS` polls of a driven pad that never reads the level they wait for.
static uint32_t IRAM_ATTR poll_cycles(uint32_t mux)
{
    REG_WRITE(GPIO_OUT1_W1TS_REG, PAD_BIT);
    REG_WRITE(GPIO_ENABLE1_W1TS_REG, PAD_BIT);
    REG_WRITE(IO_MUX_GPIO44_REG, mux);
    esp_rom_delay_us(20);
    const uint32_t ps = mask_interrupts();
    const uint32_t start = ccount();
    poll(0, POLLS);
    const uint32_t end = ccount();
    restore_interrupts(ps);
    REG_WRITE(GPIO_ENABLE1_W1TC_REG, PAD_BIT);
    return end - start;
}

void app_main(void)
{
    knob_console_wait_go();
    knob_console_start();

    const gpio_config_t io = {
        .pin_bit_mask = 1ULL << PAD,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&io));
    const uint32_t mux = REG_READ(IO_MUX_GPIO44_REG) & ~(FUN_PU | FUN_PD);
    printf("pulls ready\n");

    printf("static up %lu\n", (unsigned long)level_with(mux | FUN_PU));
    printf("static down %lu\n", (unsigned long)level_with(mux | FUN_PD));
    printf("static both %lu\n", (unsigned long)level_with(mux | FUN_PU | FUN_PD));
    printf("static none %lu\n", (unsigned long)level_with(mux));

    const uint32_t cycles = poll_cycles(mux);
    printf("poll %u %lu\n", POLLS, (unsigned long)cycles);
    const uint32_t period = (cycles + POLLS - 1) / POLLS;

    for (int k = 0; k < REPS; k++) {
        const uint32_t wait = period * (uint32_t)k / REPS;
        const uint32_t base = settle(0, mux | FUN_PD, 0, wait);
        const uint32_t fall = settle(1, mux | FUN_PD, 0, wait);
        const uint32_t rise = settle(0, mux | FUN_PU, 1, wait);
        printf("rep %d base %lu fall %lu rise %lu\n", k, (unsigned long)base, (unsigned long)fall,
               (unsigned long)rise);
    }
    REG_WRITE(IO_MUX_GPIO44_REG, mux);
    printf("PULLS DONE\n");
}
