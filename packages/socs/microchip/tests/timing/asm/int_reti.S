; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define EECR 0x3F
#define EERIE 3
#define IO(x) ((x) - 0x20)
.section .text
.global main
.section .vectors,"ax",@progbits
.global __vectors
__vectors:
jmp main
.org 0x58
jmp isr
.section .text
main:
ldi r16,0x20
ldi r17,0x00
out IO(DDRB),r16
sbi IO(EECR),EERIE
sei
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
out IO(PORTB),r16
1: nop
rjmp 1b
isr:
out IO(PORTB),r17
reti
