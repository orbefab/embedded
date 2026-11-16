const int sensePin = 2;

int last = -1;
int reports = 0;

void report(int high) {
  if (high == last) {
    return;
  }
  last = high;
  reports++;
  Serial.print(high ? "dark" : "light");
  Serial.print(' ');
  Serial.println(reports);
}

void setup() {
  pinMode(sensePin, INPUT);
  Serial.begin(115200);
  report(digitalRead(sensePin) == HIGH);
}

void loop() {
  report(digitalRead(sensePin) == HIGH);
}
