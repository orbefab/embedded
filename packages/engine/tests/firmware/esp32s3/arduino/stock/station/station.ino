// A stock Wi-Fi station: it starts the radio, scans, joins a WPA2 access point, takes an address
// over DHCP and sets the clock over SNTP.
#include <WiFi.h>
#include <time.h>

void setup() {
  Serial.begin(115200);
  Serial.println("ready");
  Serial.printf("mode %d\n", WiFi.mode(WIFI_STA));
  int found = WiFi.scanNetworks();
  Serial.printf("scan %d\n", found);
  for (int i = 0; i < found; i++) {
    Serial.printf("ap %s ch %d\n", WiFi.SSID(i).c_str(), (int)WiFi.channel(i));
  }
  WiFi.begin("orbe", "orbe-password");
  while (WiFi.status() != WL_CONNECTED) {
    delay(100);
  }
  Serial.printf("ip %s\n", WiFi.localIP().toString().c_str());
  configTime(0, 0, "pool.ntp.org");
  struct tm now;
  while (!getLocalTime(&now, 1000)) {
  }
  Serial.printf("time %04d-%02d-%02d\n", now.tm_year + 1900, now.tm_mon + 1, now.tm_mday);
}

void loop() {
  delay(1000);
}
