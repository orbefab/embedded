; PB1 (OC1A) fades an LED. Timer1 mode 1, 8-bit phase-correct PWM,
; TOP 0xFF, OCR1x copied at TOP (Table 16-4, §16.9.4). Prescaler 64,
; Table 16-5 CS = 011. COM1A = 2, non-inverting, Table 16-3.
; f = fclk / (2 * N * TOP) = 16 MHz / 32640.
; OCR1A steps by 5 every 480,000 cycles, 30 ms at 16 MHz, and the
; step reverses at 0 and 255. OCR1AH is written before OCR1AL (§16.3).
; Data-space addresses, DS40002061B Table 31-1.
; Cycle counts, DS40002198B, AVRe column.
#define DDRB 0x24
#define TCCR1A 0x80
#define TCCR1B 0x81
#define OCR1AH 0x89
#define OCR1AL 0x88
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(DDRB),1          ; 2: PB1 an output, low, at cycle 2
ldi r16,0x81            ; 1: COM1A1, WGM10. Mode 1, non-inverting.
sts TCCR1A,r16          ; 2: retires at cycle 5
ldi r16,0x03            ; 1: CS = /64
sts TCCR1B,r16          ; 2: retires at cycle 8. The first /64 tick is cycle 64.
ldi r16,0               ; 1: OCR1A starts at 0, the reset value
ldi r17,5               ; 1: the step, at cycle 10
; A pass is 480,000 cycles from one OCR1AL retire edge to the next.
; 2 + 3 + 5 * 95,995 - 1 + 4 + 12 + 5 = 480,000.
; The first store retires at 10 + 3 + 5 * 95,995 - 1 + 4 + 12 + 5 = 480,008.
loop:
ldi r24,lo8(95995)      ; 1
ldi r25,hi8(95995)      ; 1
ldi r26,hlo8(95995)     ; 1
1: subi r24,1           ; 1
sbci r25,0              ; 1
sbci r26,0              ; 1
brne 1b                 ; 2, then 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
; 12 cycles on every path, then the store. NEG flips the step, and a
; value other than 0 or 255 flips it back.
add r16,r17             ; 1
neg r17                 ; 1
cpi r16,0               ; 1
breq zero               ; 2, or 1
cpi r16,255             ; 1
breq top                ; 2, or 1
neg r17                 ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
rjmp store              ; 2
zero:
nop                     ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
rjmp store              ; 2
top:
nop                     ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
nop                     ; 1
store:
ldi r18,0               ; 1
sts OCR1AH,r18          ; 2: high byte first, §16.3
sts OCR1AL,r16          ; 2: retires two cycles later. The buffer copies at the next TOP.
rjmp loop               ; 2
