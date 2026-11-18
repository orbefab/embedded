// GPIO6 read with Arduino's analogRead and analogReadMilliVolts at the default attenuation and
// width, 11 dB and 12 bits. Eight passes of loop each print "adc <k> raw <raw> mv <mv>", then
// "ADC DONE" once.
void setup()
{
  Serial.begin(115200);
}

void loop()
{
  static int n;
  if (n < 8) {
    const uint16_t raw = analogRead(6);
    const uint32_t mv = analogReadMilliVolts(6);
    Serial.printf("adc %d raw %u mv %lu\n", n++, raw, (unsigned long)mv);
  } else if (n == 8) {
    Serial.printf("ADC DONE\n");
    n++;
  }
  delay(10);
}
