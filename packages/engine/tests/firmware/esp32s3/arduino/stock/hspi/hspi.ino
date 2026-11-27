#include <SPI.h>

// SPI (FSPI / SPI2): SCK=12, MISO=13, MOSI=11, SS=10
// hspi (HSPI / SPI3): SCK=4, MISO=5, MOSI=6, SS=7
SPIClass hspi(HSPI);

void setup() {
  Serial.begin(115200);

  SPI.begin(12, 13, 11, 10);
  SPI.setHwCs(true);
  hspi.begin(4, 5, 6, 7);
  hspi.setHwCs(true);

  uint8_t a[4] = {0x9F, 0x00, 0x00, 0x00};
  SPI.beginTransaction(SPISettings(4000000, MSBFIRST, SPI_MODE0));
  SPI.transfer(a, sizeof(a));
  SPI.endTransaction();

  uint8_t b[4] = {0x9F, 0x00, 0x00, 0x00};
  hspi.beginTransaction(SPISettings(4000000, MSBFIRST, SPI_MODE0));
  hspi.transfer(b, sizeof(b));
  hspi.endTransaction();

  Serial.printf("fspi %02x %02x %02x hspi %02x %02x %02x\n", a[1], a[2], a[3], b[1], b[2], b[3]);
  Serial.println("ready");
}

void loop() {
  delay(100);
}
