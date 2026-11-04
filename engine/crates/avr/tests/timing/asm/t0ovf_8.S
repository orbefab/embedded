; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define DDRD 0x2A
#define TCCR0A 0x44
#define TCCR0B 0x45
#define OCR0A 0x47
#define TIMSK0 0x6E
#define IO(x) ((x) - 0x20)
.section .text
.global main
.section .vectors,"ax",@progbits
.global __vectors
__vectors:
jmp main
.org 0x40
jmp isr
.section .text
main:
ldi r16,0x20
out IO(DDRB),r16
ldi r16,0x40
out IO(DDRD),r16
ldi r16,0xFF
out IO(OCR0A),r16
ldi r16,0x40
out IO(TCCR0A),r16
ldi r16,1
sts TIMSK0,r16
sei
ldi r16,2
out IO(TCCR0B),r16
2:
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
rjmp 2b
isr:
in r18,IO(PORTB)
ldi r19,0x20
eor r18,r19
out IO(PORTB),r18
reti
