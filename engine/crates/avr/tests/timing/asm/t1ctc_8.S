; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define TCCR1A 0x80
#define TCCR1B 0x81
#define OCR1AL 0x88
#define OCR1AH 0x89
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0x02
out IO(DDRB),r16
ldi r16,hi8(999)
sts OCR1AH,r16
ldi r16,lo8(999)
sts OCR1AL,r16
ldi r16,0x40
sts TCCR1A,r16
ldi r16,0x08|2
sts TCCR1B,r16
1: nop
rjmp 1b
