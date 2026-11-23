// A button on GPIO4 cycles the board's RGB LED. Each colour is printed.
const int buttonPin = 4;
const uint8_t ledPin = 38;

const uint8_t colours[][3] = {
  {40, 0, 0},
  {0, 40, 0},
  {0, 0, 40},
};
const char *names[] = {"red", "green", "blue"};
const int count = 3;

int colour = 0;
int held = 0;

void show() {
  rgbLedWrite(ledPin, colours[colour][0], colours[colour][1], colours[colour][2]);
  Serial.println(names[colour]);
}

void setup() {
  Serial.begin(115200);
  pinMode(buttonPin, INPUT_PULLUP);
  show();
}

void loop() {
  int down = digitalRead(buttonPin) == LOW;
  if (down && !held) {
    colour = (colour + 1) % count;
    show();
    held = 1;
  } else if (!down) {
    held = 0;
  }
  delay(20);
}
