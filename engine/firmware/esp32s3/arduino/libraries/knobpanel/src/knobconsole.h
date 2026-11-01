#pragma once
#include <Arduino.h>
#include "soc/rtc_cntl_reg.h"

inline void knob_console_command(int c)
{
  if (c == 'D' || c == 'R') {
    Serial.println(c == 'D' ? "KNOB REBOOT DOWNLOAD" : "KNOB REBOOT APP");
    Serial.flush();
    delay(100);
    if (c == 'D') REG_WRITE(RTC_CNTL_OPTION1_REG, RTC_CNTL_FORCE_DOWNLOAD_BOOT);
    esp_restart();
  }
}

inline void knob_console_begin()
{
  Serial.setTxBufferSize(16384);
  Serial.setTxTimeoutMs(5000);
  Serial.begin(115200);
}

inline void knob_console_wait_go()
{
  for (;;) {
    int c = Serial.read();
    if (c == 'g') return;
    knob_console_command(c);
    delay(5);
  }
}
