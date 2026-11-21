// ADC1 channel 5, GPIO6, read with IDF's one-shot driver. After 'g' on the console, for each
// attenuation 0 to 3 (0, 2.5, 6 and 11 dB) it configures the channel at 12 bits, makes the
// curve-fitting calibration of that attenuation, and reads 64 samples. Each prints
// "adc <atten> <k> <raw> <mv>", the raw code and the calibrated millivolts, or -1 for the
// millivolts when the calibration could not be made, after a line
// "cali <atten> ok|<error name>". The app ends with "ADC DONE".
#include <stdio.h>
#include "esp_adc/adc_cali.h"
#include "esp_adc/adc_cali_scheme.h"
#include "esp_adc/adc_oneshot.h"
#include "esp_err.h"
#include "knobconsole.h"

#define CHANNEL ADC_CHANNEL_5
#define SAMPLES 64

void app_main(void)
{
    knob_console_wait_go();
    knob_console_start();

    adc_oneshot_unit_handle_t unit;
    const adc_oneshot_unit_init_cfg_t init = {
        .unit_id = ADC_UNIT_1,
        .ulp_mode = ADC_ULP_MODE_DISABLE,
    };
    ESP_ERROR_CHECK(adc_oneshot_new_unit(&init, &unit));
    printf("adc ready\n");

    for (int atten = ADC_ATTEN_DB_0; atten <= ADC_ATTEN_DB_12; atten++) {
        const adc_oneshot_chan_cfg_t channel = {
            .atten = (adc_atten_t)atten,
            .bitwidth = ADC_BITWIDTH_12,
        };
        ESP_ERROR_CHECK(adc_oneshot_config_channel(unit, CHANNEL, &channel));
        adc_cali_handle_t cali = NULL;
        const adc_cali_curve_fitting_config_t curve = {
            .unit_id = ADC_UNIT_1,
            .chan = CHANNEL,
            .atten = (adc_atten_t)atten,
            .bitwidth = ADC_BITWIDTH_12,
        };
        const esp_err_t made = adc_cali_create_scheme_curve_fitting(&curve, &cali);
        printf("cali %d %s\n", atten, made == ESP_OK ? "ok" : esp_err_to_name(made));
        for (int k = 0; k < SAMPLES; k++) {
            int raw = 0;
            ESP_ERROR_CHECK(adc_oneshot_read(unit, CHANNEL, &raw));
            int mv = -1;
            if (made == ESP_OK) {
                ESP_ERROR_CHECK(adc_cali_raw_to_voltage(cali, raw, &mv));
            }
            printf("adc %d %d %d %d\n", atten, k, raw, mv);
        }
        if (made == ESP_OK) {
            ESP_ERROR_CHECK(adc_cali_delete_scheme_curve_fitting(cali));
        }
    }
    printf("ADC DONE\n");
}
