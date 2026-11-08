; Converts ADC0 against AVCC once, left adjusted, and drives its top eight
; bits on PD7 to PD0. Data-space addresses, DS40002061B Table 31-1. ADMUX and
; ADCSRA, §24.9.
#define DDRD 0x2A
#define PORTD 0x2B
#define ADCH 0x79
#define ADCSRA 0x7A
#define ADMUX 0x7C
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0xFF
out IO(DDRD),r16
ldi r16,0x60            ; REFS = 01, AVCC. ADLAR. MUX = 0, ADC0.
sts ADMUX,r16
ldi r16,0xC7            ; ADEN, ADSC, prescaler /128.
sts ADCSRA,r16
1: lds r16,ADCSRA
sbrc r16,6              ; ADSC clears when the conversion completes.
rjmp 1b
lds r16,ADCH
out IO(PORTD),r16
2: rjmp 2b
