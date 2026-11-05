; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define TCCR0B 0x45
#define TCNT0 0x46
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0x3F
out IO(DDRB),r16
nop
nop
nop
nop
ldi r16,1
out IO(TCCR0B),r16
nop
nop
nop
nop
nop
in r18,IO(TCNT0)
out IO(PORTB),r18
1: nop
rjmp 1b
