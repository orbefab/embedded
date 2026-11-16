// Reads a DHT22 on D2 every 2 s and prints the read's number, then its temperature and humidity
// to 0.1, as "n=1 t=25.0 h=50.0", or "n=1 error=timeout" or "n=1 error=checksum".
//
// The read is the one in Aosong's AM2302 manual: hold D2 low for 1.1 ms, release it to the
// pull-up, then time the sensor's response and its 40 bits. Each bit is a 50 us low and a high
// of 26 us for a 0 or 70 us for a 1, so a bit is a 1 when its high outlasts its low. The loop
// counts reads of the pin instead of calling micros(), which stops while interrupts are off.

const uint8_t DATA_BIT = 2;  // D2 is PD2
const uint16_t TIMEOUT = 0xFFFF;
const uint16_t LIMIT = 1000;  // about 400 us of reads at 16 MHz

// How many reads D2 stays at `level`, or TIMEOUT.
static uint16_t held(uint8_t level) {
  uint16_t count = 0;
  while (((PIND >> DATA_BIT) & 1) == level) {
    if (++count >= LIMIT) {
      return TIMEOUT;
    }
  }
  return count;
}

// The five bytes of one read, or the error's name.
static const char *readSensor(uint8_t bytes[5]) {
  uint16_t lows[40];
  uint16_t highs[40];
  pinMode(2, OUTPUT);
  digitalWrite(2, LOW);
  delayMicroseconds(1100);
  noInterrupts();
  pinMode(2, INPUT);
  bool ok = held(1) != TIMEOUT && held(0) != TIMEOUT && held(1) != TIMEOUT;
  for (uint8_t i = 0; ok && i < 40; i++) {
    lows[i] = held(0);
    highs[i] = held(1);
    ok = lows[i] != TIMEOUT && highs[i] != TIMEOUT;
  }
  interrupts();
  if (!ok) {
    return "timeout";
  }
  for (uint8_t i = 0; i < 5; i++) {
    bytes[i] = 0;
  }
  for (uint8_t i = 0; i < 40; i++) {
    bytes[i / 8] = (bytes[i / 8] << 1) | (highs[i] > lows[i] ? 1 : 0);
  }
  if ((uint8_t)(bytes[0] + bytes[1] + bytes[2] + bytes[3]) != bytes[4]) {
    return "checksum";
  }
  return 0;
}

static void printTenths(int16_t tenths) {
  if (tenths < 0) {
    Serial.print('-');
    tenths = -tenths;
  }
  Serial.print(tenths / 10);
  Serial.print('.');
  Serial.print(tenths % 10);
}

void setup() {
  Serial.begin(115200);
  pinMode(2, INPUT);
}

uint16_t reads = 0;

void loop() {
  delay(2000);
  uint8_t bytes[5];
  const char *error = readSensor(bytes);
  reads++;
  Serial.print("n=");
  Serial.print(reads);
  Serial.print(' ');
  if (error) {
    Serial.print("error=");
    Serial.println(error);
    return;
  }
  int16_t temperature = ((int16_t)(bytes[2] & 0x7F) << 8) | bytes[3];
  if (bytes[2] & 0x80) {
    temperature = -temperature;
  }
  Serial.print("t=");
  printTenths(temperature);
  Serial.print(" h=");
  printTenths(((int16_t)bytes[0] << 8) | bytes[1]);
  Serial.println();
}
