#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>

// 128 x 64 on Wire, with no reset pin. The module's VCC source is 0 V at the start and steps
// to 5 V during the delay. The first transmission is refused. begin works once the regulator
// is over the controller's logic minimum.
Adafruit_SSD1306 display(128, 64, &Wire, -1);

void probe() {
  Wire.beginTransmission(0x3C);
  Serial.print("TX ");
  Serial.println(Wire.endTransmission());
}

void setup() {
  Serial.begin(115200);
  Wire.begin();
  probe();
  delay(50);
  probe();
  bool ok = display.begin(SSD1306_SWITCHCAPVCC, 0x3C);
  Serial.println(ok ? "BEGIN 1" : "BEGIN 0");
}

void loop() {}
