; PB5 is an input, and its pull-up turns on at cycle 2 (DS40002061B Table 14-1).
#define PORTB 0x25
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(PORTB),5         ; 2
1: rjmp 1b
