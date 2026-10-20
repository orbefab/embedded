; Drives every pin of ports B, C, and D high. Data-space addresses,
; DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define DDRC 0x27
#define PORTC 0x28
#define DDRD 0x2A
#define PORTD 0x2B
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0xFF
out IO(DDRB),r16
out IO(PORTB),r16
out IO(DDRC),r16
out IO(PORTC),r16
out IO(DDRD),r16
out IO(PORTD),r16
1: rjmp 1b
