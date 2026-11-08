; PB5 is low at cycle 2. CLKPR then divides the clock by 2 (DS40002061B
; §9.12.2), and PB5 toggles every 100 CPU cycles. Data-space addresses,
; Table 31-1. Cycle counts, DS40002198B, AVRe column.
#define PINB 0x23
#define DDRB 0x24
#define CLKPR 0x61
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(DDRB),5          ; 2
ldi r16,0x80            ; 1: CLKPCE
sts CLKPR,r16           ; 2
ldi r16,0x01            ; 1: CLKPS = 1, divide by 2
sts CLKPR,r16           ; 2
toggle:
sbi IO(PINB),5          ; 2
ldi r16,31              ; 1
1: dec r16              ; 1
brne 1b                 ; 2, then 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
rjmp toggle             ; 2
