; PB5 toggles every 8,000,000 cycles, 0.5 s at 16 MHz. Data-space addresses,
; DS40002061B Table 31-1. Cycle counts, DS40002198B, AVRe column.
#define PINB 0x23
#define DDRB 0x24
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(DDRB),5          ; 2: PB5 an output, low, at cycle 2
toggle:
sbi IO(PINB),5          ; 2: PB5 toggles at cycle 4, then every 8,000,000
; 2 + 3 + 5 * 1,599,998 - 1 + 4 + 2 = 8,000,000 a pass.
ldi r24,lo8(1599998)    ; 1
ldi r25,hi8(1599998)    ; 1
ldi r26,hlo8(1599998)   ; 1
1: subi r24,1           ; 1
sbci r25,0              ; 1
sbci r26,0              ; 1
brne 1b                 ; 2, then 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
rjmp toggle             ; 2
