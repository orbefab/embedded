#pragma once

// Console commands the image answers on USB Serial/JTAG: 'g' starts a run that waits for it, 'R'
// restarts the app, 'D' restarts into the ROM download mode. The RX FIFO is polled directly, so
// the console's own TX path stays IDF's default. The line count is kept, as the image hashes it.
void knob_console_start(void);
void knob_console_wait_go(void);
