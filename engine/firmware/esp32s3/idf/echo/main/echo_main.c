// Installs the USB Serial/JTAG driver, waits one tick, prints "ready <t>" and echoes each
// byte as "got <b>".
#include <stdio.h>
#include <string.h>
#include "driver/usb_serial_jtag.h"
#include "esp_err.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

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

void app_main(void)
{
    usb_serial_jtag_driver_config_t config = USB_SERIAL_JTAG_DRIVER_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&config));
    vTaskDelay(1);
    int64_t t = esp_timer_get_time();
    char line[32];
    snprintf(line, sizeof line, "ready %lld\n", (long long)t);
    write_all(line);
    for (;;) {
        uint8_t byte = 0;
        if (usb_serial_jtag_read_bytes(&byte, 1, portMAX_DELAY) != 1) {
            continue;
        }
        snprintf(line, sizeof line, "got %u\n", byte);
        write_all(line);
    }
}
