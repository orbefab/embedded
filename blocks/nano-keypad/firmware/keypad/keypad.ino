#include <Keypad.h>

const byte ROWS = 4;
const byte COLS = 4;

char keys[ROWS][COLS] = {
  {'1', '2', '3', 'A'},
  {'4', '5', '6', 'B'},
  {'7', '8', '9', 'C'},
  {'*', '0', '#', 'D'}
};

// Wired to R1..R4 (PORTB: D11..D8) and C1..C4 (PORTD: D5..D2) on keypad-4x4.
// Keeping rows and columns on separate AVR ports avoids reading hi-Z columns via PINx.
byte rowPins[ROWS] = {11, 10, 9, 8};
byte colPins[COLS] = {5, 4, 3, 2};

Keypad keypad = Keypad(makeKeymap(keys), rowPins, colPins, ROWS, COLS);

void setup() {
  Serial.begin(115200);
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.println("ready");
}

void loop() {
  char key = keypad.getKey();
  if (key) {
    Serial.print("key=");
    Serial.println(key);
  }
  KeyState state = keypad.getState();
  digitalWrite(LED_BUILTIN, (state == PRESSED || state == HOLD) ? HIGH : LOW);
}
