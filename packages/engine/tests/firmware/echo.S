; PB5 follows PD2. Data-space addresses, DS40002061B Table 31-1.
#define PIND 0x29
#define DDRB 0x24
#define PORTB 0x25
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(DDRB),5
1: sbis IO(PIND),2
cbi IO(PORTB),5
sbic IO(PIND),2
sbi IO(PORTB),5
rjmp 1b
