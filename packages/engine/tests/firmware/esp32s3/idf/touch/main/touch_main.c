// The touch controller on I2C0, SDA GPIO9 and SCL GPIO10, at address 0x15. TP_RST on GPIO8 is
// held low for 10 ms and then released. The app reads register 0xA7 and prints "id <byte>",
// then reads register 0x02 for five bytes ten times, 33 ms apart, and prints "poll f=<fingers>".
// A falling edge on TP_INT, GPIO7, reads those five bytes again and prints
// "touch f=<n> x=<x> y=<y>".
#include <stdio.h>
#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "knobconsole.h"

#define PIN_INT GPIO_NUM_7
#define PIN_RST GPIO_NUM_8
#define PIN_SDA GPIO_NUM_9
#define PIN_SCL GPIO_NUM_10
#define ADDRESS 0x15
#define POLLS 10
#define GAP_MS 33

static QueueHandle_t edges;
static i2c_master_dev_handle_t dev;

static void read_reg(uint8_t reg, uint8_t *buf, size_t n)
{
    ESP_ERROR_CHECK(i2c_master_transmit_receive(dev, &reg, 1, buf, n, 100));
}

static void IRAM_ATTR on_int(void *arg)
{
    uint8_t one = 1;
    BaseType_t woken = pdFALSE;
    xQueueSendFromISR(edges, &one, &woken);
    portYIELD_FROM_ISR(woken);
}

void app_main(void)
{
    knob_console_wait_go();
    knob_console_start();

    const gpio_config_t reset = {
        .pin_bit_mask = 1ULL << PIN_RST,
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&reset));
    gpio_set_level(PIN_RST, 0);
    vTaskDelay(pdMS_TO_TICKS(10));
    gpio_set_level(PIN_RST, 1);

    const i2c_master_bus_config_t bus_config = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = PIN_SDA,
        .scl_io_num = PIN_SCL,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
        .flags.enable_internal_pullup = true,
    };
    i2c_master_bus_handle_t bus;
    ESP_ERROR_CHECK(i2c_new_master_bus(&bus_config, &bus));
    const i2c_device_config_t dev_config = {
        .dev_addr_length = I2C_ADDR_BIT_LEN_7,
        .device_address = ADDRESS,
        .scl_speed_hz = 400000,
    };
    ESP_ERROR_CHECK(i2c_master_bus_add_device(bus, &dev_config, &dev));

    uint8_t id;
    read_reg(0xA7, &id, 1);
    printf("id %02x\n", id);

    for (int n = 0; n < POLLS; n++) {
        uint8_t buf[5];
        read_reg(0x02, buf, 5);
        printf("poll f=%u\n", buf[0]);
        if (n + 1 < POLLS) {
            vTaskDelay(pdMS_TO_TICKS(GAP_MS));
        }
    }

    edges = xQueueCreate(8, sizeof(uint8_t));
    const gpio_config_t line = {
        .pin_bit_mask = 1ULL << PIN_INT,
        .mode = GPIO_MODE_INPUT,
        .pull_up_en = GPIO_PULLUP_ENABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_NEGEDGE,
    };
    ESP_ERROR_CHECK(gpio_config(&line));
    ESP_ERROR_CHECK(gpio_install_isr_service(0));
    ESP_ERROR_CHECK(gpio_isr_handler_add(PIN_INT, on_int, NULL));

    for (;;) {
        uint8_t one;
        xQueueReceive(edges, &one, portMAX_DELAY);
        uint8_t buf[5];
        read_reg(0x02, buf, 5);
        unsigned x = ((buf[1] & 0x0f) << 8) | buf[2];
        unsigned y = ((buf[3] & 0x0f) << 8) | buf[4];
        printf("touch f=%u x=%u y=%u\n", buf[0], x, y);
    }
}
