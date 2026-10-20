; Read ADC0 against AVCC and print the code as decimal on TXD, one line
; per reading. Data-space addresses, DS40002061B Table 31-1.
; USART0, §20. ADC, §24. Cycle counts, DS40002198B, AVRe column.
;
; U2X0 stays clear. UBRR0 is 103, Table 20-6 at 16 MHz for 9600 baud.
; A bit is 16 * (103 + 1) cycles, §20.3.1 and Table 20-1, which is 104 us.
; The ADC prescaler is /128, so the ADC clock is 125 kHz, inside the
; 50 to 200 kHz band of §24.4.
#define UCSR0A 0xC0
#define UCSR0B 0xC1
#define UCSR0C 0xC2
#define UBRR0L 0xC4
#define UBRR0H 0xC5
#define UDR0 0xC6
#define ADCL 0x78
#define ADCH 0x79
#define ADCSRA 0x7A
#define ADMUX 0x7C
.section .text
.global main
main:
ldi r16,0x06            ; UCSZ 8-bit, no parity, 1 stop. UMSEL async.
sts UCSR0C,r16
ldi r16,0               ; §20.11.5: UBRR0H, then UBRR0L.
sts UBRR0H,r16
ldi r16,103
sts UBRR0L,r16
ldi r16,0x08            ; TXEN0. TXD idles high from here.
sts UCSR0B,r16
ldi r16,0x40            ; REFS AVCC, ADLAR clear, MUX ADC0.
sts ADMUX,r16
ldi r16,0x87            ; ADEN, prescaler /128.
sts ADCSRA,r16
loop:
ldi r16,0xC7            ; ADSC.
sts ADCSRA,r16
1: lds r16,ADCSRA
sbrc r16,6              ; ADSC clears when the conversion completes.
rjmp 1b
lds r16,ADCL            ; §24.9.3: ADCL, then ADCH.
lds r17,ADCH
rcall print
ldi r16,0x0D
rcall emit
ldi r16,0x0A
rcall emit
ldi r16,0x40            ; Clear TXC, then wait until this line has left.
sts UCSR0A,r16
2: lds r16,UCSR0A
sbrs r16,6
rjmp 2b
; 1 + 1 + 3999 * (2 + 2) + (2 + 1) = 16001 cycles, 1.0000625 ms at 16 MHz.
ldi r24,lo8(4000)       ; 1
ldi r25,hi8(4000)       ; 1
3: sbiw r24,1            ; 2
brne 3b                 ; 2, then 1
rjmp loop

; Poll UDRE0 (§20.11.2 bit 5), then write the byte in r16.
emit:
lds r18,UCSR0A
sbrs r18,5
rjmp emit
sts UDR0,r16
ret

; r17:r16 is 0 to 1023. Digits with no leading zeros.
print:
clr r20
clr r21
clr r22
4: ldi r18,lo8(1000)
ldi r19,hi8(1000)
cp r16,r18
cpc r17,r19
brlo 5f
sub r16,r18
sbc r17,r19
inc r20
rjmp 4b
5: ldi r19,0
cpi r16,100
cpc r17,r19
brlo 6f
subi r16,100
sbc r17,r19
inc r21
rjmp 5b
6: cpi r16,10
brlo 7f
subi r16,10
inc r22
rjmp 6b
7: mov r23,r16
clr r19
tst r20
breq 8f
mov r16,r20
rcall digit
ldi r19,1
8: tst r21
brne 9f
tst r19
breq 10f
9: mov r16,r21
rcall digit
ldi r19,1
10: tst r22
brne 11f
tst r19
breq 12f
11: mov r16,r22
rcall digit
12: mov r16,r23
rcall digit
ret

digit:
subi r16,-'0'
rjmp emit
