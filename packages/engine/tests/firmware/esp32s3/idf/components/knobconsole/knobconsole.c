#include <stdio.h>
#include "knobconsole.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "hal/usb_serial_jtag_ll.h"
#include "soc/rtc_cntl_reg.h"

static int knob_getc(void)
{
    uint8_t c;
    if (usb_serial_jtag_ll_rxfifo_data_available() && usb_serial_jtag_ll_read_rxfifo(&c, 1) == 1) {
        return c;
    }
    return -1;
}

static void knob_command(int c)
{
    if (c != 'D' && c != 'R') {
        return;
    }
    printf(c == 'D' ? "KNOB REBOOT DOWNLOAD\n" : "KNOB REBOOT APP\n");
    fflush(stdout);
    vTaskDelay(pdMS_TO_TICKS(100));
    if (c == 'D') {
        REG_WRITE(RTC_CNTL_OPTION1_REG, RTC_CNTL_FORCE_DOWNLOAD_BOOT);
    }
    esp_restart();
}

static void knob_console_task(void *arg)
{
    for (;;) {
        knob_command(knob_getc());
        vTaskDelay(pdMS_TO_TICKS(20));
    }
}

void knob_console_start(void)
{
    xTaskCreate(knob_console_task, "knobcon", 2048, NULL, 1, NULL);
}

void knob_console_wait_go(void)
{
    for (;;) {
        int c = knob_getc();
        if (c == 'g') {
            return;
        }
        knob_command(c);
        vTaskDelay(pdMS_TO_TICKS(10));
    }
}
