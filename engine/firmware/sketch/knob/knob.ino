// Prints the A and B fall counts each time either line falls.
const int aPin = 2;
const int bPin = 3;

unsigned int aCount = 0;
unsigned int bCount = 0;
int aLast = HIGH;
int bLast = HIGH;

void setup() {
  pinMode(aPin, INPUT);
  pinMode(bPin, INPUT);
  Serial.begin(115200);
}

void loop() {
  int a = digitalRead(aPin);
  int b = digitalRead(bPin);
  bool fell = false;
  if (a == LOW && aLast == HIGH) {
    aCount++;
    fell = true;
  }
  if (b == LOW && bLast == HIGH) {
    bCount++;
    fell = true;
  }
  aLast = a;
  bLast = b;
  if (fell) {
    Serial.print(aCount);
    Serial.print(' ');
    Serial.println(bCount);
  }
}
