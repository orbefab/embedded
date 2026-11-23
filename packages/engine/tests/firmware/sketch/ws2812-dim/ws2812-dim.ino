static const uint8_t FRAME[] = {
    8, 0, 0, 8, 0, 0, 8, 0, 0, 8, 0, 0,
    8, 0, 0, 8, 0, 0, 8, 0, 0, 8, 0, 0,
};

static void send(const uint8_t *data, uint8_t count) {
  asm volatile(
      "1:\n"
      "ld r18, %a0+\n"
      "ldi r19, 8\n"
      "2:\n"
      "sbi %[port], %[bit]\n"
      "lsl r18\n"
      "brcs 3f\n"
      "nop\n"
      "cbi %[port], %[bit]\n"
      "nop\n"
      "nop\n"
      "nop\n"
      "rjmp 4f\n"
      "3:\n"
      "nop\n"
      "nop\n"
      "nop\n"
      "nop\n"
      "nop\n"
      "cbi %[port], %[bit]\n"
      "4:\n"
      "dec r19\n"
      "brne 5f\n"
      "dec %1\n"
      "brne 1b\n"
      "rjmp 6f\n"
      "5:\n"
      "nop\n"
      "nop\n"
      "nop\n"
      "rjmp 2b\n"
      "6:\n"
      : "+x"(data), "+r"(count)
      : [port] "I"(_SFR_IO_ADDR(PORTB)), [bit] "I"(0)
      : "r18", "r19", "memory");
}

void setup() {
  uint8_t sreg = SREG;
  cli();
  DDRB |= _BV(0);
  PORTB &= ~_BV(0);
  send(FRAME, sizeof FRAME);
  delayMicroseconds(300);
  SREG = sreg;
}

void loop() {}
