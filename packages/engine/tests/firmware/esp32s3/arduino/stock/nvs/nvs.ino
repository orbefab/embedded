#include <nvs.h>
#include <nvs_flash.h>

void setup() {
  Serial.begin(115200);

  esp_err_t init1 = nvs_flash_init();
  nvs_handle_t h = 0;
  esp_err_t open1 = nvs_open("cfg", NVS_READWRITE, &h);
  esp_err_t set1 = nvs_set_i32(h, "count", 7);
  esp_err_t commit1 = nvs_commit(h);
  nvs_close(h);

  esp_err_t erase = nvs_flash_erase();
  esp_err_t init2 = nvs_flash_init();

  esp_err_t open2 = nvs_open("cfg", NVS_READWRITE, &h);
  int32_t missing = 0;
  esp_err_t gone = nvs_get_i32(h, "count", &missing);
  esp_err_t set2 = nvs_set_i32(h, "count", 99);
  esp_err_t set3 = nvs_set_str(h, "tag", "after");
  esp_err_t commit2 = nvs_commit(h);
  nvs_close(h);

  esp_err_t open3 = nvs_open("cfg", NVS_READONLY, &h);
  int32_t gotCount = 0;
  esp_err_t get1 = nvs_get_i32(h, "count", &gotCount);
  char tag[16] = {0};
  size_t tagLen = sizeof(tag);
  esp_err_t get2 = nvs_get_str(h, "tag", tag, &tagLen);
  nvs_close(h);

  bool ok = init1 == ESP_OK && open1 == ESP_OK && set1 == ESP_OK && commit1 == ESP_OK
    && erase == ESP_OK && init2 == ESP_OK && open2 == ESP_OK
    && gone == ESP_ERR_NVS_NOT_FOUND && set2 == ESP_OK && set3 == ESP_OK && commit2 == ESP_OK
    && open3 == ESP_OK && get1 == ESP_OK && get2 == ESP_OK;

  Serial.printf("nvs ok=%d count=%ld tag=%s\n", ok ? 1 : 0, (long)gotCount, tag);
}

void loop() {
  delay(100);
}
