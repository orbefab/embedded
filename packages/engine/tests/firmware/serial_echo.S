; Echo each byte RXD receives on TXD. "1" turns PB5 on and "0" turns it off.
; Data-space addresses, DS40002061B Table 31-1. Vectors, Table 12-6.
; Cycle counts, DS40002198B, AVRe column.
;
; U2X0 stays clear. UBRR0 is 103, Table 20-6 at 16 MHz for 9600 baud.
; The idle loop is `rjmp` and touches no flag, so the RX vector does not save SREG.
; Table 12-6 has 26 vectors of two words. Only USART_RX is enabled. The others
; are `reti`, so a reader who enables one does not fall into `rx`.
#define UCSR0A 0xC0
#define UCSR0B 0xC1
#define UCSR0C 0xC2
#define UBRR0L 0xC4
#define UBRR0H 0xC5
#define UDR0 0xC6
#define DDRB 0x24
#define PORTB 0x25
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
rjmp start
nop
.rept 17
rjmp unused
nop
.endr
.org 0x48               ; USART_RX, word 0x0024
rjmp rx
nop
.rept 7
rjmp unused
nop
.endr
unused:
reti
start:
ldi r16,0               ; U2X0 clear.
sts UCSR0A,r16
ldi r16,0x06            ; UCSZ 8-bit, no parity, 1 stop.
sts UCSR0C,r16
ldi r16,0               ; §20.11.5: UBRR0H, then UBRR0L.
sts UBRR0H,r16
ldi r16,103
sts UBRR0L,r16
ldi r16,0x98            ; RXCIE0, RXEN0, TXEN0.
sts UCSR0B,r16
sbi IO(DDRB),5
sei
1: rjmp 1b
rx:
lds r16,UDR0
cpi r16,'1'
brne 2f
sbi IO(PORTB),5
2: cpi r16,'0'
brne 3f
cbi IO(PORTB),5
3: lds r17,UCSR0A
sbrs r17,5              ; UDRE0.
rjmp 3b
sts UDR0,r16
reti
