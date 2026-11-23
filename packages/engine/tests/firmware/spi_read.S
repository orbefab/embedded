; An SPI master on PB2 to PB5 at fosc/16 sends 0x9F and three more bytes, and puts each of
; the three bytes it read on PORTD in turn, about 50 cycles apart. Data-space addresses,
; DS40002061B Table 31-1, and SPCR bits, section 19.5.1.
#define DDRB 0x24
#define PORTB 0x25
#define DDRD 0x2A
#define PORTD 0x2B
#define SPCR 0x4C
#define SPSR 0x4D
#define SPDR 0x4E
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
sbi IO(PORTB),2         ; CS, PB2, high
ldi r16,0x2C            ; PB2, PB3 MOSI and PB5 SCK out
out IO(DDRB),r16
ldi r16,0xFF
out IO(DDRD),r16
ldi r16,0x51            ; SPE, MSTR and SPR0
out IO(SPCR),r16
cbi IO(PORTB),2
ldi r16,0x9F
rcall xfer
ldi r16,0
rcall xfer
mov r20,r16
ldi r16,0
rcall xfer
mov r21,r16
ldi r16,0
rcall xfer
mov r22,r16
sbi IO(PORTB),2
out IO(PORTD),r20
rcall wait
out IO(PORTD),r21
rcall wait
out IO(PORTD),r22
1: rjmp 1b
; Sends r16 and returns the byte read in r16.
xfer:
out IO(SPDR),r16
1: in r17,IO(SPSR)
sbrs r17,7
rjmp 1b
in r16,IO(SPDR)
ret
wait:
ldi r18,16
1: dec r18
brne 1b
ret
