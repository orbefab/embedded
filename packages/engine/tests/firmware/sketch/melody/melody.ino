// Random melodies for the buzzer's hz test, drawn from a fixed xorshift32 seed. D12 toggles
// just before each event, so the test finds when it began. Notes play on D8 with tone(),
// PWM on D3, D5 or D9 with analogWrite. packages/session/tests/buzzer.rs draws the same
// events from the same seed.

static uint32_t state = 0x2545F491UL;
static unsigned int last = 440;
static int8_t pwm = -1;
static uint8_t marker = LOW;

static uint32_t draw() {
  state ^= state << 13;
  state ^= state >> 17;
  state ^= state << 5;
  return state;
}

static uint16_t below(uint16_t n) { return draw() % n; }

// 31 Hz to 3966 Hz, even in octaves, and 31 and 4000 themselves now and then.
static unsigned int pitch() {
  uint16_t pick = below(16);
  if (pick == 0) return 31;
  if (pick == 1) return 4000;
  uint32_t octave = below(7);
  uint32_t step = below(1024);
  return (31UL << octave) * (1024 + step) / 1024;
}

// More than an octave up or down from the last note, inside 31 Hz to 4000 Hz.
static unsigned int leap() {
  uint32_t ratio = 2048 + below(6144);
  uint32_t f = below(2) ? (uint32_t)last * ratio / 1024 : (uint32_t)last * 1024 / ratio;
  if (f < 31) f = 31;
  if (f > 4000) f = 4000;
  return f;
}

// 5 ms to 300 ms, and 5 ms to 40 ms three times in four, so a run holds many changes.
static unsigned long length() { return below(4) ? 5 + below(36) : 5 + below(296); }

// 0 ms to 400 ms, and 0 ms to 40 ms three times in four.
static unsigned long rest() { return below(4) ? below(41) : below(401); }

static void mark() {
  if (pwm >= 0) {
    analogWrite(pwm, 0);
    pwm = -1;
  }
  marker = !marker;
  digitalWrite(12, marker);
}

void setup() {
  pinMode(3, OUTPUT);
  pinMode(5, OUTPUT);
  pinMode(8, OUTPUT);
  pinMode(9, OUTPUT);
  pinMode(12, OUTPUT);
  delay(20);
}

void loop() {
  uint16_t kind = below(16);
  if (kind < 6) {
    last = pitch();
    mark();
    tone(8, last);
    delay(length());
  } else if (kind < 8) {
    last = pitch();
    unsigned long play = length();
    mark();
    tone(8, last, play);
    delay(length());
    // A tone() call as the timed tone expires races its interrupt's noTone(), which leaves
    // Timer2 in PWM mode toggling D8 at some 58 kHz.
    noTone(8);
  } else if (kind == 8) {
    mark();
    tone(8, last);
    delay(length());
  } else if (kind == 9) {
    last = leap();
    mark();
    tone(8, last);
    delay(length());
  } else if (kind < 12) {
    mark();
    noTone(8);
    delay(rest());
  } else {
    static const uint8_t pins[] = {3, 5, 9};
    static const uint8_t duties[] = {1, 4, 128, 254};
    uint8_t pin = pins[below(3)];
    uint8_t duty = duties[below(4)];
    mark();
    noTone(8);
    analogWrite(pin, duty);
    pwm = pin;
    delay(length());
  }
}
