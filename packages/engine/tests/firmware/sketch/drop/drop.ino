// D9 powers a coil that tone() on D8 switches at 31 Hz, then goes low while D8 plays on.

void setup() {
  Serial.begin(115200);
  pinMode(8, OUTPUT);
  pinMode(9, OUTPUT);
}

void loop() {
  Serial.println("drop");
  delay(20);
  digitalWrite(9, HIGH);
  tone(8, 31);
  delay(110);
  digitalWrite(9, LOW);
  delay(60);
  noTone(8);
  delay(60);
}
