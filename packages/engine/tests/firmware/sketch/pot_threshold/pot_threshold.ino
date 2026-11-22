// Lights the LED on D13 while A0 reads above half scale, and prints the reading each time it
// moves by more than 8 counts.
const int ledPin = 13;
const int threshold = 512;
int printed = -100;

void setup() {
  pinMode(ledPin, OUTPUT);
  Serial.begin(115200);
}

void loop() {
  int value = analogRead(A0);
  digitalWrite(ledPin, value > threshold ? HIGH : LOW);
  if (abs(value - printed) > 8) {
    Serial.println(value);
    printed = value;
  }
  delay(5);
}
