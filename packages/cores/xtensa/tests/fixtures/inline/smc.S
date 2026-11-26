    .include "vectors.inc"
    .org 0x3d0
    # Two versions of the word at `patch`: `addi.n a6, a6, 1; addi.n a7, a7, 1` and
    # `addi.n a6, a6, 2; addi.n a7, a7, 3`.
one:
    addi.n a6, a6, 1
    addi.n a7, a7, 1
two:
    addi.n a6, a6, 2
    addi.n a7, a7, 3
data:   .word 0x40208000
patchp: .word patch
    .org 0x400
    .global start
start:
    l32r a1, data
    movi a2, 24
pass:
    # The word to write: `two` while bit 2 of the pass equals the control word at
    # data + 64, else `one`.
    l32r a8, one
    l32r a9, two
    l32i a10, a1, 64
    extui a11, a2, 2, 1
    xor a11, a11, a10
    moveqz a8, a9, a11
    # Every fourth pass writes it over the two ops right after the store, from inside the
    # straight run the rest of the pass runs in. The other passes write it to data + 96.
    l32r a4, patchp
    extui a12, a2, 0, 2
    addi a13, a1, 96
    movnez a4, a13, a12
    # Pads `patch` to a word.
    nop
    nop.n
    s32i a8, a4, 0
patch:
    addi.n a6, a6, 1
    addi.n a7, a7, 1
    s32i a6, a1, 0
    s32i a7, a1, 4
    addi a2, a2, -1
    bnez a2, pass
end:
    j end
