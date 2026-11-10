; Data-space addresses, DS40002061B Table 31-1.
#define DDRB 0x24
#define PORTB 0x25
#define GPIOR0 0x3E
#define IO(x) ((x) - 0x20)
.section .text
.global main
main:
ldi r16,0x20
ldi r17,0x00
out IO(DDRB),r16
push r18

out IO(PORTB),r16
nop
out IO(PORTB),r17

out IO(PORTB),r16
add r18,r19
out IO(PORTB),r17

out IO(PORTB),r16
adiw r24,1
out IO(PORTB),r17

out IO(PORTB),r16
sbiw r24,1
out IO(PORTB),r17

out IO(PORTB),r16
mul r18,r19
out IO(PORTB),r17

out IO(PORTB),r16
movw r20,r18
out IO(PORTB),r17

out IO(PORTB),r16
rjmp 1f
1:
out IO(PORTB),r17

out IO(PORTB),r16
jmp 1f
1:
out IO(PORTB),r17
ldi r30,lo8(pm(1f))
ldi r31,hi8(pm(1f))
out IO(PORTB),r16
ijmp
1:
out IO(PORTB),r17
sez
out IO(PORTB),r16
breq 1f
1:
out IO(PORTB),r17
clz
out IO(PORTB),r16
breq 1f
1:
out IO(PORTB),r17
ldi r18,1
ldi r19,2
out IO(PORTB),r16
cpse r18,r19
nop
out IO(PORTB),r17
ldi r18,1
ldi r19,1
out IO(PORTB),r16
cpse r18,r19
nop
out IO(PORTB),r17
ldi r18,1
ldi r19,1
out IO(PORTB),r16
cpse r18,r19
jmp 0
out IO(PORTB),r17
ldi r18,1
out IO(PORTB),r16
sbrs r18,0
nop
out IO(PORTB),r17
ldi r18,1
out IO(PORTB),r16
sbrs r18,0
lds r0,0x100
out IO(PORTB),r17
ldi r18,1
out IO(PORTB),r16
sbrc r18,0
nop
out IO(PORTB),r17
ldi r18,1
out IO(GPIOR0),r18
out IO(PORTB),r16
sbis IO(GPIOR0),0
nop
out IO(PORTB),r17
ldi r18,1
out IO(GPIOR0),r18
out IO(PORTB),r16
sbis IO(GPIOR0),0
sts 0x100,r0
out IO(PORTB),r17
ldi r18,1
out IO(GPIOR0),r18
out IO(PORTB),r16
sbic IO(GPIOR0),0
nop
out IO(PORTB),r17
ldi r26,0x00
ldi r27,0x01
out IO(PORTB),r16
ld r18,X
out IO(PORTB),r17
ldi r26,0x00
ldi r27,0x01
out IO(PORTB),r16
ld r18,X+
out IO(PORTB),r17
ldi r26,0x08
ldi r27,0x01
out IO(PORTB),r16
ld r18,-X
out IO(PORTB),r17
ldi r28,0x10
ldi r29,0x01
out IO(PORTB),r16
ldd r18,Y+3
out IO(PORTB),r17

out IO(PORTB),r16
lds r18,0x0100
out IO(PORTB),r17
ldi r26,0x00
ldi r27,0x01
out IO(PORTB),r16
st X,r18
out IO(PORTB),r17
ldi r28,0x10
ldi r29,0x01
out IO(PORTB),r16
st -Y,r18
out IO(PORTB),r17
ldi r30,0x20
ldi r31,0x01
out IO(PORTB),r16
std Z+5,r18
out IO(PORTB),r17

out IO(PORTB),r16
sts 0x0100,r18
out IO(PORTB),r17

out IO(PORTB),r16
push r18
out IO(PORTB),r17

out IO(PORTB),r16
pop r18
out IO(PORTB),r17
ldi r30,lo8(table)
ldi r31,hi8(table)
out IO(PORTB),r16
lpm
out IO(PORTB),r17
ldi r30,lo8(table)
ldi r31,hi8(table)
out IO(PORTB),r16
lpm r18,Z+
out IO(PORTB),r17

out IO(PORTB),r16
rcall sub_ret
out IO(PORTB),r17

out IO(PORTB),r16
call sub_ret
out IO(PORTB),r17
ldi r30,lo8(pm(sub_ret))
ldi r31,hi8(pm(sub_ret))
out IO(PORTB),r16
icall
out IO(PORTB),r17

out IO(PORTB),r16
in r18,IO(GPIOR0)
out IO(PORTB),r17

out IO(PORTB),r16
out IO(GPIOR0),r18
out IO(PORTB),r17

out IO(PORTB),r16
sbi IO(GPIOR0),1
out IO(PORTB),r17

out IO(PORTB),r16
cbi IO(GPIOR0),1
out IO(PORTB),r17

out IO(PORTB),r16
ldi r18,10
1: dec r18
brne 1b
out IO(PORTB),r17
2: nop
rjmp 2b
sub_ret: ret
table: .byte 1,2,3,4
