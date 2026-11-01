void setup()
{
  Serial.begin(115200);
}

void loop()
{
  static int n;
  Serial.printf("tick %d core %d\n", n++, xPortGetCoreID());
  int c = Serial.read();
  if (c >= 0) Serial.printf("got %d\n", c);
  delay(100);
}
