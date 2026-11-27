#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>

// BMP280 on Wire (I2C0: SDA GPIO8, SCL GPIO9) at 0x76;
// 128 x 64 SSD1306 OLED on Wire1 (I2C1: SDA GPIO17, SCL GPIO18) at 0x3C.
Adafruit_SSD1306 display(128, 64, &Wire1, -1);

static uint8_t bmpRead8(uint8_t reg) {
  Wire.beginTransmission(0x76);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) {
    return 0;
  }
  if (Wire.requestFrom((uint8_t)0x76, (uint8_t)1) != 1) {
    return 0;
  }
  return Wire.read();
}

void setup() {
  Serial.begin(115200);
  Wire.begin(8, 9);
  Wire1.begin(17, 18);

  // The OLED module holds RES# low through 4.7 kohm and 10 uF for about 76 ms after power-on.
  delay(100);

  uint8_t id = bmpRead8(0xD0);
  Wire.beginTransmission(0x76);
  Wire.write(0xF4);
  Wire.write(0x25);  // forced mode, 1x temp, 1x pressure
  uint8_t err = Wire.endTransmission();
  delay(10);

  uint32_t rawPress = 0;
  uint32_t rawTemp = 0;
  Wire.beginTransmission(0x76);
  Wire.write(0xF7);
  if (Wire.endTransmission(false) == 0 && Wire.requestFrom((uint8_t)0x76, (uint8_t)6) == 6) {
    uint8_t b[6];
    for (int i = 0; i < 6; i++) {
      b[i] = Wire.read();
    }
    rawPress = ((uint32_t)b[0] << 12) | ((uint32_t)b[1] << 4) | (b[2] >> 4);
    rawTemp = ((uint32_t)b[3] << 12) | ((uint32_t)b[4] << 4) | (b[5] >> 4);
  }
  Serial.printf("bmp id %02x err %u raw %lu %lu\n", id, err, (unsigned long)rawPress, (unsigned long)rawTemp);

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
  display.setCursor(8, 32);
  display.printf("id %02x", id);
  display.display();
  Serial.println("drawn");
}

void loop() {
}
