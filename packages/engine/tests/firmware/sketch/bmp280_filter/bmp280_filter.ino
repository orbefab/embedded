// Configures a BMP280 at 0x76 by writing sleep mode to ctrl_meas (0xF4),
// then config (0xF5) with FILTER_X16 and 500 ms standby (0x90), then normal
// mode with osrs_t x2 and osrs_p x16 (0x57). Prints compensated temperature
// in hundredths of a degree C during the first standby and after each cycle.
#include <Wire.h>

static const uint8_t ADDR = 0x76;
static uint16_t dig_T1;
static int16_t dig_T2, dig_T3;
static uint32_t started_ms;

static void writeReg(uint8_t reg, uint8_t val) {
  Wire.beginTransmission(ADDR);
  Wire.write(reg);
  Wire.write(val);
  Wire.endTransmission();
}

static void readRegs(uint8_t reg, uint8_t *buf, uint8_t len) {
  Wire.beginTransmission(ADDR);
  Wire.write(reg);
  Wire.endTransmission(false);
  Wire.requestFrom(ADDR, len);
  for (uint8_t i = 0; i < len && Wire.available(); i++) {
    buf[i] = Wire.read();
  }
}

static int32_t readTempC100() {
  uint8_t buf[3];
  readRegs(0xFA, buf, 3);
  int32_t adc_T = ((int32_t)buf[0] << 12) | ((int32_t)buf[1] << 4) | (buf[2] >> 4);
  int32_t var1 = ((((adc_T >> 3) - ((int32_t)dig_T1 << 1))) * ((int32_t)dig_T2)) >> 11;
  int32_t var2 =
      (((((adc_T >> 4) - ((int32_t)dig_T1)) * ((adc_T >> 4) - ((int32_t)dig_T1))) >> 12) *
       ((int32_t)dig_T3)) >>
      14;
  int32_t t_fine = var1 + var2;
  return (t_fine * 5 + 128) >> 8;
}

static void printReading(const char *tag, int32_t c100) {
  Serial.print(tag);
  Serial.print('=');
  if (c100 < 0) {
    Serial.print('-');
    c100 = -c100;
  }
  Serial.print(c100 / 100);
  Serial.print('.');
  int32_t frac = c100 % 100;
  if (frac < 10) Serial.print('0');
  Serial.println(frac);
}

static void waitUntil(uint32_t offset_ms) {
  while ((uint32_t)(millis() - started_ms) < offset_ms) {}
}

void setup() {
  Serial.begin(38400);
  Wire.begin();
  uint8_t cal[6];
  readRegs(0x88, cal, 6);
  dig_T1 = (uint16_t)cal[0] | ((uint16_t)cal[1] << 8);
  dig_T2 = (int16_t)((uint16_t)cal[2] | ((uint16_t)cal[3] << 8));
  dig_T3 = (int16_t)((uint16_t)cal[4] | ((uint16_t)cal[5] << 8));

  // Write sleep mode to ctrl_meas (0xF4), then config (0xF5) with
  // t_sb = 100 (500 ms, Table 11) and filter = 100 (coeff 16, Table 6),
  // then normal mode with osrs_t = x2, osrs_p = x16 (t_measure = 37.5 ms, Table 13).
  writeReg(0xF4, 0x00);
  writeReg(0xF5, 0x90);
  writeReg(0xF4, 0x57);
  started_ms = millis();

  // Sample 0 finishes at t_measure = 37.5 ms.
  waitUntil(60);
  printReading("s0", readTempC100());

  // Inside the 500 ms standby (ends at 537.5 ms), before sample 1 starts: still sample 0.
  waitUntil(500);
  printReading("sb", readTempC100());

  // Cycle period is t_measure (37.5 ms) + t_sb (500 ms) = 537.5 ms.
  // Samples 1..4 finish at 575 ms, 1112.5 ms, 1650 ms, 2187.5 ms.
  static const uint16_t offsets[4] = {600, 1138, 1675, 2213};
  static const char *const tags[4] = {"s1", "s2", "s3", "s4"};
  for (uint8_t i = 0; i < 4; i++) {
    waitUntil(offsets[i]);
    printReading(tags[i], readTempC100());
  }
}

void loop() {}
