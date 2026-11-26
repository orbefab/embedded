    .include "vectors.inc"
    .org 0x3e0
data:   .word 0x40208000
seed:   .word 0x9e3779b1
fnp:    .word fn2
jxp:    .word jxt
    .org 0x400
    .global start
start:
    l32r a1, data
    l32r a3, seed
    movi a2, 40
pass:
    mull a4, a2, a3
    srli a5, a4, 7
    xor a5, a5, a4
    addx2 a6, a5, a4
    addx4 a7, a6, a5
    addx8 a8, a7, a6
    sub a9, a8, a5
    subx2 a10, a9, a6
    subx4 a11, a10, a7
    subx8 a12, a11, a8
    addi a13, a12, -77
    addi.n a13, a13, 5
    addmi a14, a13, 0x1200
    and a15, a14, a5
    or a15, a15, a9
    neg a15, a15
    s32i a6, a1, 16
    s32i a7, a1, 20
    s32i a8, a1, 24
    s32i a9, a1, 28
    s32i a10, a1, 32
    s32i a11, a1, 36
    s32i a12, a1, 40
    s32i a13, a1, 44
    s32i a14, a1, 48
    s32i a15, a1, 52
    movi a6, -1000
    movi.n a7, 33
    mov.n a8, a15
    extui a9, a4, 3, 2
    moveqz a8, a6, a9
    movnez a7, a5, a9
    minu a10, a4, a5
    maxu a11, a4, a5
    nsau a12, a5
    slli a13, a4, 9
    srai a14, a4, 11
    srli a15, a4, 15
    s32i a6, a1, 16
    s32i a7, a1, 20
    s32i a8, a1, 24
    s32i a9, a1, 28
    s32i a10, a1, 32
    s32i a11, a1, 36
    s32i a12, a1, 40
    s32i a13, a1, 44
    s32i a14, a1, 48
    s32i a15, a1, 52
    ssr a2
    sll a6, a4
    srl a7, a5
    sra a8, a4
    src a9, a4, a5
    ssl a2
    sll a10, a5
    ssa8l a2
    src a11, a5, a4
    ssai 13
    srl a12, a4
    s32i a6, a1, 16
    s32i a7, a1, 20
    s32i a8, a1, 24
    s32i a9, a1, 28
    s32i a10, a1, 32
    s32i a11, a1, 36
    s32i a12, a1, 40
    s32i a4, a1, 0
    s16i a5, a1, 6
    s8i a6, a1, 9
    s32i.n a7, a1, 12
    memw
    l32i a13, a1, 0
    l16ui a14, a1, 6
    l16si a15, a1, 6
    memw
    l8ui a6, a1, 9
    l32i.n a7, a1, 12
    l32r a8, seed
    rsr.ccount a9
    nop
    nop.n
    s32i a13, a1, 44
    s32i a14, a1, 48
    s32i a15, a1, 52
    s32i a6, a1, 16
    s32i a7, a1, 20
    s32i a8, a1, 24
    s32i a9, a1, 28
    extui a10, a4, 4, 3
    extui a11, a4, 9, 3
    movi a15, 0
    beqz a10, 1f
    addi a15, a15, 1
1:  beqz.n a11, 1f
    addi a15, a15, 2
1:  bnez a10, 1f
    addi a15, a15, 4
1:  bnez.n a11, 1f
    addi a15, a15, 8
1:  bgez a4, 1f
    addi a15, a15, 16
1:  bltz a5, 1f
    addi a15, a15, 32
1:  beq a10, a11, 1f
    addi a15, a15, 64
1:  bne a10, a11, 1f
    addi a15, a15, 127
1:  bge a4, a5, 1f
    addi a15, a15, 3
1:  blt a4, a5, 1f
    addi a15, a15, 5
1:  bgeu a4, a5, 1f
    addi a15, a15, 7
1:  bltu a10, a11, 1f
    addi a15, a15, 9
1:  bany a10, a11, 1f
    addi a15, a15, 11
1:  bnone a10, a11, 1f
    addi a15, a15, 13
1:  bbci a4, 5, 1f
    addi a15, a15, 15
1:  bbsi a4, 6, 1f
    addi a15, a15, 17
1:  beqi a10, 4, 1f
    addi a15, a15, 19
1:  bnei a11, 2, 1f
    addi a15, a15, 21
1:  bgei a10, 3, 1f
    addi a15, a15, 23
1:  blti a11, 5, 1f
    addi a15, a15, 25
1:  bgeui a10, 6, 1f
    addi a15, a15, 27
1:  bltui a11, 3, 1f
    addi a15, a15, 29
1:  call0 fn1
    l32r a6, fnp
    callx0 a6
    j 1f
    addi a15, a15, 100
1:  l32r a6, jxp
    jx a6
    addi a15, a15, 99
jxt:
    s32i a15, a1, 52
    addi a2, a2, -1
    bnez a2, pass
end:
    j end
    .align 4
fn1:
    addi a15, a15, 31
    ret
    .align 4
fn2:
    addi a15, a15, 37
    ret.n
