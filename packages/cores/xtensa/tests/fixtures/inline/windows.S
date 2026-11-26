    .include "vectors.inc"
    .org 0x3e0
stack:  .word 0x4020c000
save:   .word 0x4020d000
fibp:   .word fib8
    .org 0x400
    .global start
start:
    l32r a1, stack
    # The start frame never ran `entry`, so give the handlers that spill it a save area.
    l32r a3, save
    addi a4, a1, -16
    s32i a3, a4, 0
    s32i a3, a4, 4
    movi a6, 11
    call4 fib4
    s32i a6, a1, 0
    movi a10, 13
    call8 fib8
    s32i a10, a1, 4
    movi a14, 9
    call12 fib12
    s32i a14, a1, 8
end:
    j end

    # fib(n) by call4: the argument and the result in a2.
    .align 4
fib4:
    entry a1, 32
    bltui a2, 2, 1f
    addi a6, a2, -1
    call4 fib4
    mov.n a3, a6
    addi a6, a2, -2
    call4 fib4
    add.n a2, a3, a6
    s32i a2, a1, 0
1:  retw.n

    # fib(n) by call8 and callx8.
    .align 4
fib8:
    entry a1, 32
    bltui a2, 2, 1f
    addi a10, a2, -1
    call8 fib8
    mov.n a4, a10
    addi a10, a2, -2
    l32r a8, fibp
    callx8 a8
    add.n a2, a4, a10
    s32i a2, a1, 0
1:  retw

    # fib(n) by call12.
    .align 4
fib12:
    entry a1, 48
    bltui a2, 2, 1f
    addi a14, a2, -1
    call12 fib12
    mov.n a4, a14
    addi a14, a2, -2
    call12 fib12
    add.n a2, a4, a14
    s32i a2, a1, 0
1:  retw.n
