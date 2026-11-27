// The LiquidCrystal HelloWorld on an 8-bit bus: the library's ten-pin constructor puts DB0 to
// DB7 on D9 to D2, with RS on D12 and E on D11.
#include <LiquidCrystal.h>

LiquidCrystal lcd(12, 11, 9, 8, 7, 6, 5, 4, 3, 2);

void setup() {
  lcd.begin(16, 2);
  lcd.print("hello, world!");
}

void loop() {
  lcd.setCursor(0, 1);
  lcd.print(millis() / 1000);
}
