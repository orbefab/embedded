; Data-space addresses, DS40002061B Table 31-1.
#define DDRD 0x2A
#define TCCR0A 0x44
#define TCCR0B 0x45
#define OCR0A 0x47
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0x40
out IO(DDRD),r16
ldi r16,99
out IO(OCR0A),r16
ldi r16,0x42
out IO(TCCR0A),r16
nop
nop
nop
ldi r16,2
out IO(TCCR0B),r16
1: nop
rjmp 1b
