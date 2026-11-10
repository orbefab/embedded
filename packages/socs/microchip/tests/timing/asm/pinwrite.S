; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0x20
out IO(DDRB),r16
out IO(PORTB),r16
cbi IO(PORTB),5
ldi r26,0x25
ldi r27,0x00
st X,r16
ldi r17,0
sts 0x25,r17
1: nop
rjmp 1b
