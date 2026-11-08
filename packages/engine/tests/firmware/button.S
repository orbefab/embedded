; PD2 is an input with its pull-up on. PB5 copies the inverse of PIND2, so the
; LED on PB5 is on while a button from PD2 to ground is held.
; Data-space addresses, DS40002061B Table 31-1. Pull-up, Table 14-1.
; Cycle counts, DS40002198B, AVRe column.
#define PIND 0x29
#define DDRD 0x2A
#define PORTD 0x2B
#define DDRB 0x24
#define PORTB 0x25
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
cbi IO(DDRD),2          ; 2: PD2 an input
sbi IO(PORTD),2         ; 2: pull-up on at cycle 4
sbi IO(DDRB),5          ; 2: PB5 an output, low, at cycle 6
; 1 + 1 + 1 + 1 + 1 + 1 + 2 = 8 cycles a pass. OUT retires 6 cycles after IN.
loop:
in r16,IO(PIND)         ; 1
com r16                 ; 1
bst r16,2               ; 1
in r17,IO(PORTB)        ; 1
bld r17,5               ; 1
out IO(PORTB),r17       ; 1
rjmp loop               ; 2
