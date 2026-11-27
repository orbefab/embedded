const int PIN_X = A0;
const int PIN_Y = A1;
const int PIN_SW = 2;

int lastX = -100;
int lastY = -100;
int lastBtn = -1;

void setup() {
  pinMode(PIN_SW, INPUT_PULLUP);
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
}

void loop() {
  int x = analogRead(PIN_X);
  int y = analogRead(PIN_Y);
  int btn = digitalRead(PIN_SW) == LOW ? 1 : 0;
  digitalWrite(LED_BUILTIN, btn ? HIGH : LOW);
  if (abs(x - lastX) >= 4 || abs(y - lastY) >= 4 || btn != lastBtn) {
    Serial.print("x=");
    Serial.print(x);
    Serial.print(" y=");
    Serial.print(y);
    Serial.print(" btn=");
    Serial.println(btn);
    lastX = x;
    lastY = y;
    lastBtn = btn;
  }
  delay(20);
}
