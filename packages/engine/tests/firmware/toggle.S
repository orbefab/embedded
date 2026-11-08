; PB5 is low at cycle 2 and toggles at cycle 4, then every 100 cycles.
; Data-space addresses, DS40002061B Table 31-1. Cycle counts, DS40002198B,
; AVRe column.
#define PINB 0x23
#define DDRB 0x24
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(DDRB),5          ; 2
toggle:
sbi IO(PINB),5          ; 2
; 2 + 1 + 3 * 31 - 1 + 3 + 2 = 100 a pass.
ldi r16,31              ; 1
1: dec r16              ; 1
brne 1b                 ; 2, then 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
rjmp toggle             ; 2
