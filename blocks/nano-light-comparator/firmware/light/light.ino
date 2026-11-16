const int sensePin = 2;

int last = -1;

void report(int high) {
  if (high == last) {
    return;
  }
  last = high;
  Serial.println(high ? "dark" : "light");
}

void setup() {
  pinMode(sensePin, INPUT);
  Serial.begin(115200);
  report(digitalRead(sensePin) == HIGH);
}

void loop() {
  report(digitalRead(sensePin) == HIGH);
}
