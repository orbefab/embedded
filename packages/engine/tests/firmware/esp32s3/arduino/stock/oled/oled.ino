#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>

// 128 x 64 on Wire, SDA on GPIO8 and SCL on GPIO9, with no reset pin.
Adafruit_SSD1306 display(128, 64, &Wire, -1);

void setup() {
  Serial.begin(115200);
  // The module holds RES# low through 4.7 kohm and 10 uF for about 76 ms after power-on.
  delay(100);
  if (!display.begin(SSD1306_SWITCHCAPVCC, 0x3C)) {
    Serial.println("no display");
    return;
  }
  display.clearDisplay();
  display.drawRect(0, 0, 128, 64, SSD1306_WHITE);
  display.setTextSize(2);
  display.setTextColor(SSD1306_WHITE);
  display.setCursor(8, 8);
  display.print("orbe");
  display.display();
  Serial.println("drawn");
}

void loop() {
}
