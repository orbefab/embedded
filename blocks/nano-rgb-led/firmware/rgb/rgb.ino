const int PIN_R = 9;
const int PIN_G = 10;
const int PIN_B = 11;

void setColor(uint8_t r, uint8_t g, uint8_t b) {
  analogWrite(PIN_R, r);
  analogWrite(PIN_G, g);
  analogWrite(PIN_B, b);
}

void setup() {
  pinMode(PIN_R, OUTPUT);
  pinMode(PIN_G, OUTPUT);
  pinMode(PIN_B, OUTPUT);
}

void loop() {
  setColor(255, 0, 0);    // red
  delay(400);
  setColor(255, 128, 0);  // orange (PWM mix)
  delay(400);
  setColor(0, 255, 0);    // green
  delay(400);
  setColor(0, 255, 255);  // cyan
  delay(400);
  setColor(0, 0, 255);    // blue
  delay(400);
  setColor(255, 0, 128);  // magenta (PWM mix)
  delay(400);
}
