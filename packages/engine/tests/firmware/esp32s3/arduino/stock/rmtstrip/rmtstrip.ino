const int pin = 4;
const int pixels = 8;

rmt_data_t bits[pixels * 24];

const uint32_t first[pixels] = {
  0xff0000, 0x00ff00, 0x0000ff, 0xffff00,
  0x00ffff, 0xff00ff, 0x102030, 0x804020,
};
const uint32_t second[pixels] = {
  0x010203, 0x400000, 0x004000, 0x000040,
  0x808080, 0x0a141e, 0xc0ffee, 0xffffff,
};

// Each colour is 0xRRGGBB. The pixels take green, then red, then blue, most significant bit
// first, at 10 MHz: a 1 is 800 ns high and 400 ns low, a 0 is 400 ns high and 800 ns low.
void show(const uint32_t *colors) {
  int i = 0;
  for (int p = 0; p < pixels; p++) {
    uint32_t c = colors[p];
    uint32_t grb = ((c >> 8) & 0xff) << 16 | ((c >> 16) & 0xff) << 8 | (c & 0xff);
    for (int b = 23; b >= 0; b--) {
      bool one = grb & (1UL << b);
      bits[i].level0 = 1;
      bits[i].duration0 = one ? 8 : 4;
      bits[i].level1 = 0;
      bits[i].duration1 = one ? 4 : 8;
      i++;
    }
  }
  rmtWrite(pin, bits, pixels * 24, RMT_WAIT_FOR_EVER);
}

void setup() {
  Serial.begin(115200);
  rmtInit(pin, RMT_TX_MODE, RMT_MEM_NUM_BLOCKS_1, 10000000);
  show(first);
  Serial.println("first");
  delay(20);
  show(second);
  Serial.println("second");
}

void loop() {
  delay(1000);
}
