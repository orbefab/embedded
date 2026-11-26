#include <LittleFS.h>

void setup() {
  Serial.begin(115200);

  bool mounted1 = LittleFS.begin(true);
  File w = LittleFS.open("/note.txt", FILE_WRITE);
  size_t n1 = w ? w.print("hello ") : 0;
  if (w) w.close();

  File a = LittleFS.open("/note.txt", FILE_APPEND);
  size_t n2 = a ? a.print("flash") : 0;
  if (a) a.close();

  LittleFS.end();

  bool mounted2 = LittleFS.begin(false);
  File r = LittleFS.open("/note.txt", FILE_READ);
  String text = r ? r.readString() : "";
  if (r) r.close();
  LittleFS.end();

  Serial.printf(
    "littlefs mount=%d%d wrote=%u text=%s\n",
    mounted1 ? 1 : 0,
    mounted2 ? 1 : 0,
    (unsigned)(n1 + n2),
    text.c_str()
  );
}

void loop() {
  delay(100);
}
