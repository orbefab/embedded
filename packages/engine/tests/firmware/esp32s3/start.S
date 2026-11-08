/* The vectors, the reset entry and the CCOMPARE1 interrupt of blink.c.
 *
 * The level-3 interrupt toggles GPIO21 with one store to GPIO_OUT, so every edge comes the same
 * number of cycles after its CCOMPARE1 match. It notes GPIO_IN as it found it and SYSTIMER
 * after the store, and moves CCOMPARE1 on by one period. */

    .file   "start.S"

#define GPIO_BASE       0x60004000
#define GPIO_OUT        0x04
#define GPIO_IN         0x3c
#define SYSTIMER_BASE   0x60023000
#define UNIT0_OP        0x04
#define UNIT0_VALUE_LO  0x44
#define UPDATE          (1 << 30)
#define VALUE_VALID_BIT 29
#define LED_BIT         21

    .section .vectors, "ax"
    .global _vectors
_vectors:
    .org 0x1c0
level3_vector:
    wsr     a0, excsave3
    j       level3_handler

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

    .align  4
level3_handler:
    movi    a0, isr_save
    s32i    a2, a0, 0
    s32i    a3, a0, 4
    s32i    a4, a0, 8
    s32i    a5, a0, 12
    movi    a2, GPIO_BASE
    l32i    a3, a2, GPIO_IN
    l32i    a4, a2, GPIO_OUT
    movi    a5, 1 << LED_BIT
    xor     a4, a4, a5
    s32i    a4, a2, GPIO_OUT
    /* a3 = the level GPIO21 read before the edge, a2 = the edge's index. */
    extui   a3, a3, LED_BIT, 1
    movi    a4, edges
    l32i    a2, a4, 0
    addx4   a5, a2, a4
    s32i    a3, a5, 4
    addi    a3, a2, 1
    s32i    a3, a4, 0
    movi    a3, SYSTIMER_BASE
    movi    a4, UPDATE
    s32i    a4, a3, UNIT0_OP
3:  l32i    a4, a3, UNIT0_OP
    bbci    a4, VALUE_VALID_BIT, 3b
    l32i    a4, a3, UNIT0_VALUE_LO
    movi    a3, stamps
    addx4   a5, a2, a3
    s32i    a4, a5, 0
    rsr     a2, ccompare1
    movi    a3, PERIOD
    add     a2, a2, a3
    wsr     a2, ccompare1
    movi    a0, isr_save
    l32i    a2, a0, 0
    l32i    a3, a0, 4
    l32i    a4, a0, 8
    l32i    a5, a0, 12
    rsr     a0, excsave3
    rfi     3

    .bss
    .align  4
isr_save:
    .space  16
