    .include "vectors.inc"
    # No loop body is aligned. The assembler warns, and the core runs them alike.
    .org 0x3e0
data:   .word 0x40208000
    .org 0x400
    .global start
start:
    l32r a1, data
    movi a2, 30
pass:
    # A loop whose count is 0 to 7, skipped at 0, of ops run in line.
    extui a3, a2, 0, 3
    loopnez a3, 1f
    addi a4, a4, 3
    l32i a5, a1, 0
    add a5, a5, a4
    s32i a5, a1, 0
    xor a7, a7, a5
1:
    # A loop of one op.
    movi a3, 4
    loop a3, 1f
    addi a9, a9, 1
1:
    # A loop that ends in a branch out of it, taken on odd passes.
    movi a3, 5
    loop a3, 2f
    addi a8, a8, 1
    s32i a8, a1, 4
    bbsi a2, 0, 3f
2:  addi a10, a10, 1
3:  s32i a10, a1, 8
    # A loop the count makes negative, which loopgtz skips.
    addi a3, a2, -20
    loopgtz a3, 1f
    slli a11, a11, 1
    addi a11, a11, 1
1:  s32i a11, a1, 12
    s32i a9, a1, 16
    s32i a7, a1, 20
    addi a2, a2, -1
    bnez a2, pass
end:
    j end
