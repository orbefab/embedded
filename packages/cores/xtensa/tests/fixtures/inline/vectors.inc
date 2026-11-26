    .text
    .org 0x0
    s32e a0, a5, -16
    s32e a1, a5, -12
    s32e a2, a5, -8
    s32e a3, a5, -4
    rfwo
    .org 0x40
    l32e a0, a5, -16
    l32e a1, a5, -12
    l32e a2, a5, -8
    l32e a3, a5, -4
    rfwu
    .org 0x80
    s32e a0, a9, -16
    l32e a0, a1, -12
    s32e a1, a9, -12
    s32e a2, a9, -8
    s32e a3, a9, -4
    s32e a4, a0, -32
    s32e a5, a0, -28
    s32e a6, a0, -24
    s32e a7, a0, -20
    rfwo
    .org 0xc0
    l32e a1, a9, -12
    l32e a0, a9, -16
    l32e a7, a1, -12
    l32e a2, a9, -8
    l32e a4, a7, -32
    l32e a3, a9, -4
    l32e a5, a7, -28
    l32e a6, a7, -24
    l32e a7, a7, -20
    rfwu
    .org 0x100
    s32e a0, a13, -16
    l32e a0, a1, -12
    s32e a1, a13, -12
    s32e a2, a13, -8
    s32e a3, a13, -4
    s32e a4, a0, -48
    s32e a5, a0, -44
    s32e a6, a0, -40
    s32e a7, a0, -36
    s32e a8, a0, -32
    s32e a9, a0, -28
    s32e a10, a0, -24
    s32e a11, a0, -20
    rfwo
    .org 0x140
    l32e a1, a13, -12
    l32e a0, a13, -16
    l32e a11, a1, -12
    l32e a2, a13, -8
    l32e a4, a11, -48
    l32e a8, a11, -32
    l32e a3, a13, -4
    l32e a5, a11, -44
    l32e a6, a11, -40
    l32e a7, a11, -36
    l32e a9, a11, -28
    l32e a10, a11, -24
    l32e a11, a11, -20
    rfwu
    # Level 1: the timer. Each match sets the next one 61 cycles on.
    .org 0x300
    xsr.excsave1 a2
    rsr.ccount a2
    addi a2, a2, 61
    wsr.ccompare0 a2
    xsr.excsave1 a2
    rfe
