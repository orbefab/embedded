/* The vectors and the reset entry of pattern.c.
 *
 * The level-3 interrupt is pattern.c's CCOMPARE1 delay. It writes CCOMPARE1 back, which clears
 * the match, and returns to the WAITI it woke. */

    .file   "pattern_start.S"

    .section .vectors, "ax"
    .global _vectors
_vectors:
    .org 0x1c0
level3_vector:
    wsr     a0, excsave3
    rsr     a0, ccompare1
    wsr     a0, ccompare1
    rsr     a0, excsave3
    rfi     3

    .org 0x300
kernel_vector:
    j       hang

    .org 0x340
user_vector:
    j       hang

    .org 0x3c0
double_vector:
    j       hang

    .text
    .global _start
    .align  4
_start:
    movi    a0, 0
    movi    a1, _stack_top
    movi    a2, _vectors
    wsr     a2, vecbase
    movi    a2, 0
    wsr     a2, intenable
    wsr     a2, ps
    rsync
    movi    a2, _bss_start
    movi    a3, _bss_end
1:  bgeu    a2, a3, 2f
    s32i    a0, a2, 0
    addi    a2, a2, 4
    j       1b
2:  call0   main
hang:
    waiti   15
    j       hang
