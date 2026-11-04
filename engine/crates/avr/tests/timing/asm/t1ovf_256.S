; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define TIMSK1 0x6F
#define TCCR1B 0x81
#define IO(x) ((x) - 0x20)
.section .text
.global main
.section .vectors,"ax",@progbits
.global __vectors
__vectors:
jmp main
.org 0x34
jmp isr
.section .text
main:
ldi r16,0x20
out IO(DDRB),r16
ldi r16,1
sts TIMSK1,r16
sei
ldi r16,4
sts TCCR1B,r16
2: nop
rjmp 2b
isr:
in r18,IO(PORTB)
ldi r19,0x20
eor r18,r19
out IO(PORTB),r18
reti
