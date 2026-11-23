const int btnPin = 5;

void setup() {
  Serial.begin(115200);
  pinMode(btnPin, INPUT_PULLUP);
  Serial.println("ready");
}

void loop() {
  Serial.println(digitalRead(btnPin) == HIGH ? "open" : "down");
  delay(50);
}
