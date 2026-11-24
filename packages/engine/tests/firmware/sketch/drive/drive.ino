// Phases a buzzer test finds by the line printed just before each one.
// Timer2 plays tone() on D8. Timer1 PWM is D9, Timer0 PWM is D5. In `cut`, D9 powers the coil.

void setup() {
  Serial.begin(115200);
  pinMode(8, OUTPUT);
  pinMode(5, OUTPUT);
  pinMode(9, OUTPUT);
}

void mark(const char *name) {
  noTone(8);
  digitalWrite(8, LOW);
  analogWrite(5, 0);
  analogWrite(9, 0);
  Serial.println(name);
  delay(20);
}

void loop() {
  mark("cut");
  digitalWrite(9, HIGH);
  tone(8, 440);
  delay(100);
  digitalWrite(9, LOW);
  delay(60);
  noTone(8);
  delay(60);

  mark("khz");
  tone(8, 1000, 120);
  delay(120);

  mark("two");
  tone(8, 2000, 80);
  delay(80);

  mark("four");
  tone(8, 4000, 80);
  delay(80);

  mark("low");
  tone(8, 31, 180);
  delay(180);

  mark("clicks");
  for (int i = 0; i < 6; i++) {
    digitalWrite(8, HIGH);
    delayMicroseconds(200);
    digitalWrite(8, LOW);
    delay(40);
  }

  mark("notes");
  tone(8, 440, 40);
  delay(40);
  tone(8, 880, 40);
  delay(40);
  tone(8, 2048, 40);
  delay(40);

  mark("up");
  tone(8, 440, 70);
  delay(70);
  tone(8, 880, 70);
  delay(70);

  mark("down");
  tone(8, 880, 80);
  delay(80);
  tone(8, 440, 120);
  delay(120);

  mark("melody");
  tone(8, 440, 30);
  delay(30);
  tone(8, 880, 30);
  delay(30);
  tone(8, 660, 30);
  delay(30);

  mark("start");
  tone(8, 600, 80);
  delay(80);

  mark("pwm");
  analogWrite(9, 128);
  delay(30);
  analogWrite(9, 10);
  delay(50);
  analogWrite(9, 250);
  delay(50);
  analogWrite(9, 0);
  delay(10);
  analogWrite(5, 128);
  delay(40);

  mark("brief");
  tone(8, 440, 50);
  delay(90);

  mark("off");
  tone(8, 880);
  delay(40);
  noTone(8);
  delay(30);

  mark("dc");
  digitalWrite(8, HIGH);
  delay(40);

  mark("high");
  tone(8, 10000, 40);
  delay(40);

  mark("ultra");
  tone(8, 65535, 40);
  delay(40);

  mark("flat");
  analogWrite(8, 128);
  delay(40);

  mark("rest");
  tone(8, 1760);
  delay(40);
  noTone(8);
  delay(40);
  tone(8, 440, 50);
  delay(90);
  tone(8, 1000);
  delay(40);

  mark("fall");
  tone(8, 1760);
  delay(40);
  tone(8, 220);
  delay(40);
  tone(8, 880);
  delay(40);
  tone(8, 110);
  delay(60);
  tone(8, 1000);
  delay(40);
  tone(8, 31);
  delay(150);

  mark("scale");
  tone(8, 440, 30);
  delay(30);
  tone(8, 494, 30);
  delay(30);
  tone(8, 523, 30);
  delay(30);

  mark("climb");
  tone(8, 31);
  delay(100);
  tone(8, 1000);
  delay(30);
  tone(8, 110);
  delay(50);
  tone(8, 220);
  delay(40);

  delay(300);
}
