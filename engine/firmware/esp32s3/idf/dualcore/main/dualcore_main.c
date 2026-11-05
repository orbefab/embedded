// One task on each core prints "core <c> tick <k>" every 10 ms, twenty times, then "done".
#include <stdio.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "knobconsole.h"

#define TICKS 20
#define GAP_MS 10

static void worker(void *arg)
{
    (void)arg;
    int core = (int)xPortGetCoreID();
    for (int k = 0; k < TICKS; k++) {
        printf("core %d tick %d\n", core, k);
        vTaskDelay(pdMS_TO_TICKS(GAP_MS));
    }
    printf("done\n");
    vTaskDelete(NULL);
}

void app_main(void)
{
    knob_console_start();
    xTaskCreatePinnedToCore(worker, "c0", 2048, NULL, 1, NULL, 0);
    xTaskCreatePinnedToCore(worker, "c1", 2048, NULL, 1, NULL, 1);
}
