#include <Preferences.h>

const int btnPin = 5;
Preferences prefs;
bool lastDown = false;

void setup() {
  Serial.begin(115200);
  pinMode(btnPin, INPUT_PULLUP);

  prefs.begin("app", false);
  uint32_t count = prefs.getUInt("count", 0) + 1;
  prefs.putUInt("count", count);
  prefs.putInt("signed", -42);
  prefs.putString("name", "orbe");
  uint8_t raw[4] = {0x12, 0x34, 0x56, 0x78};
  prefs.putBytes("raw", raw, sizeof(raw));
  prefs.end();

  prefs.begin("app", true);
  uint32_t gotCount = prefs.getUInt("count", 0);
  int32_t gotSigned = prefs.getInt("signed", 0);
  String gotName = prefs.getString("name", "");
  uint8_t gotRaw[4] = {0};
  size_t gotLen = prefs.getBytes("raw", gotRaw, sizeof(gotRaw));
  prefs.end();

  Serial.printf(
    "prefs count=%lu signed=%ld name=%s raw=%02x%02x%02x%02x len=%u\n",
    (unsigned long)gotCount,
    (long)gotSigned,
    gotName.c_str(),
    gotRaw[0],
    gotRaw[1],
    gotRaw[2],
    gotRaw[3],
    (unsigned)gotLen
  );
}

void loop() {
  bool down = digitalRead(btnPin) == LOW;
  if (down && !lastDown) {
    prefs.begin("app", false);
    uint32_t count = prefs.getUInt("count", 0) + 1;
    prefs.putUInt("count", count);
    prefs.end();

    prefs.begin("app", true);
    uint32_t gotCount = prefs.getUInt("count", 0);
    prefs.end();
    Serial.printf("press count=%lu\n", (unsigned long)gotCount);
  }
  lastDown = down;
  delay(10);
}
