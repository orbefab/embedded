/* Xtensa configuration-specific ISA information.

   Customer ID=15128; Build=0x90f1f; Copyright (c) 2003-2021 Tensilica Inc.

   Permission is hereby granted, free of charge, to any person obtaining
   a copy of this software and associated documentation files (the
   "Software"), to deal in the Software without restriction, including
   without limitation the rights to use, copy, modify, merge, publish,
   distribute, sublicense, and/or sell copies of the Software, and to
   permit persons to whom the Software is furnished to do so, subject to
   the following conditions:

   The above copyright notice and this permission notice shall be included
   in all copies or substantial portions of the Software.

   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
   EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
   MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
   IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
   CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
   TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
   SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.  */

// Generated from xtensa_esp32s3/binutils/bfd/xtensa-modules.c
// commit dd1cf19f6eb327a9db51043439974a6de13f5c7f
// sha256 a54135d8e85996f7aac0e31b10b021ee26d4db1b6cf893eef081560a816c4f7a
// Do not edit. Regenerate with generate.py.
#![allow(dead_code)]
#![allow(nonstandard_style)]
#![allow(unused_parens)]
#![allow(clippy::all)]

use super::op::{Op, SlotResult, OPCODE_COUNT};

pub const MAX_VISIBLE: usize = 8;

pub static OP_CLASS: [u16; OPCODE_COUNT] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    63, 64, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 75, 76,
    77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 87, 87, 87, 87, 87,
    87, 87, 88, 88, 88, 89, 89, 89, 89, 90, 90, 91, 91, 92, 92, 92,
    92, 92, 92, 92, 92, 92, 92, 92, 92, 93, 93, 93, 93, 94, 95, 96,
    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 106, 107, 108, 108, 108, 108,
    109, 109, 110, 111, 112, 113, 114, 115, 116, 117, 117, 117, 117, 118, 119, 120,
    121, 121, 122, 123, 124, 125, 126, 127, 128, 128, 128, 129, 130, 131, 132, 133,
    134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149,
    150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165,
    166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181,
    182, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197,
    198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213,
    214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, 229,
    230, 231, 232, 233, 234, 235, 236, 237, 238, 240, 240, 241, 242, 242, 243, 243,
    243, 243, 243, 243, 243, 243, 244, 244, 244, 244, 245, 245, 245, 245, 246, 246,
    246, 246, 247, 247, 247, 247, 247, 247, 247, 247, 248, 248, 248, 248, 248, 248,
    248, 248, 249, 249, 249, 249, 249, 249, 249, 249, 250, 250, 250, 250, 250, 250,
    250, 250, 251, 251, 251, 251, 251, 251, 251, 251, 252, 252, 252, 252, 252, 252,
    252, 252, 253, 253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264, 265,
    266, 267, 268, 269, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281,
    282, 283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297,
    298, 299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313,
    314, 315, 316, 317, 318, 319, 320, 320, 320, 320, 320, 321, 321, 322, 322, 323,
    323, 324, 324, 325, 326, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337,
    338, 339, 340, 341, 341, 341, 342, 343, 344, 344, 344, 345, 346, 347, 348, 349,
    350, 350, 350, 350, 351, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361,
    362, 362, 362, 362, 366, 367, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 368, 369, 370, 371, 0, 0, 0, 0, 0,
    0, 239, 239, 363, 364, 365, 372, 373, 374, 375, 376, 377, 378, 379, 380, 381,
    382, 383, 384, 385, 386, 387, 388, 389, 390, 391, 392, 393, 394, 395, 396, 397,
    398, 399, 400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410, 411, 412, 413,
    414, 415, 416, 417, 418, 419, 420, 421, 422, 423, 424, 425, 426, 427, 428, 429,
    430, 431, 432, 433, 434, 435, 436, 437, 438, 439, 440, 441, 442, 443, 444, 445,
    446, 447, 448, 449, 450, 451, 452, 453, 454, 455, 456, 457, 458, 459, 460, 461,
    462, 463, 464, 465, 466, 467, 468, 469, 470, 471, 472, 473, 474, 475, 476, 477,
    478, 479, 480, 481, 482, 483, 484, 485, 486, 487, 488, 489, 490, 491, 492, 493,
    494, 495, 496, 497, 498, 499, 500, 501, 502, 503, 504, 505, 506, 507, 508, 509,
    510, 511, 512, 513, 514, 515, 516, 517, 518, 519, 520, 521, 522, 523, 524, 525,
    526, 527, 528, 529, 530, 531, 532, 533, 534, 535, 536, 537, 538, 539, 540, 541,
    542, 543, 544, 545, 546, 547, 548, 549, 550, 551, 552, 553, 554, 555, 556, 557,
    558, 559, 560, 561, 562, 563, 564, 565, 566, 567, 568, 569, 570, 571, 572, 573,
    574, 575, 576, 577, 578, 579, 580, 581, 582, 583, 584, 585, 586, 587, 588, 589,
    590, 591, 592, 593, 594, 595, 596, 597, 598, 599, 600, 601, 602, 603, 604, 605,
    606, 607, 608, 609, 610, 611, 612, 613, 614, 615, 616, 617, 618, 619, 620, 621,
    622, 623, 624, 625, 626, 627, 628, 629,
];

static CONST_TBL_AI4C_0: [u32; 17] = [0xffffffff, 0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xa, 0xb, 0xc, 0xd, 0xe, 0xf, 0x0];
static CONST_TBL_B4C_0: [u32; 17] = [0xffffffff, 0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0xa, 0xc, 0x10, 0x20, 0x40, 0x80, 0x100, 0x0];
static CONST_TBL_B4CU_0: [u32; 17] = [0x8000, 0x10000, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8, 0xa, 0xc, 0x10, 0x20, 0x40, 0x80, 0x100, 0x0];

fn sem_id(val: u32) -> u32 {
    val
}

fn rtoa_id(val: u32, _pc: u32) -> u32 {
    val
}

fn sem_mr_0(mut val: u32) -> u32 {
    val = val.wrapping_add(0x2u32);
    val
}

fn sem_soffsetx4(mut val: u32) -> u32 {
    let soffsetx4_in_0: u32 = ((val) & (0x3ffffu32));
    let soffsetx4_out_0: u32 = (0x4u32).wrapping_add(((((((((soffsetx4_in_0) << (0xeu32))) as i32) >> (0xeu32)) as u32)) << (0x2u32)));
    val = soffsetx4_out_0;
    val
}

fn sem_uimm12x8(mut val: u32) -> u32 {
    let uimm12x8_in_0: u32 = ((val) & (0xfffu32));
    let uimm12x8_out_0: u32 = ((uimm12x8_in_0) << (0x3u32));
    val = uimm12x8_out_0;
    val
}

fn sem_simm4(mut val: u32) -> u32 {
    let simm4_in_0: u32 = ((val) & (0xfu32));
    let simm4_out_0: u32 = ((((((simm4_in_0) << (0x1cu32))) as i32) >> (0x1cu32)) as u32);
    val = simm4_out_0;
    val
}

fn sem_ar(val: u32) -> u32 {
    val
}

fn sem_ar_0(val: u32) -> u32 {
    val
}

fn sem_ar_1(val: u32) -> u32 {
    val
}

fn sem_ar_2(val: u32) -> u32 {
    val
}

fn sem_ar_3(val: u32) -> u32 {
    val
}

fn sem_ar_4(val: u32) -> u32 {
    val
}

fn sem_immrx4(mut val: u32) -> u32 {
    let immrx4_in_0: u32 = ((val) & (0xfu32));
    let immrx4_out_0: u32 = ((((((0xfffffffu32) << (0x4u32))) | (immrx4_in_0))) << (0x2u32));
    val = immrx4_out_0;
    val
}

fn sem_lsi4x4(mut val: u32) -> u32 {
    let lsi4x4_in_0: u32 = ((val) & (0xfu32));
    let lsi4x4_out_0: u32 = ((lsi4x4_in_0) << (0x2u32));
    val = lsi4x4_out_0;
    val
}

fn sem_simm7(mut val: u32) -> u32 {
    let simm7_in_0: u32 = ((val) & (0x7fu32));
    let simm7_out_0: u32 = (((((((((((((simm7_in_0) >> (0x6u32))) & (0x1u32))) & (((((simm7_in_0) >> (0x5u32))) & (0x1u32))))).wrapping_neg()) & (0x1ffffffu32))) << (0x7u32))) | (simm7_in_0));
    val = simm7_out_0;
    val
}

fn sem_uimm6(mut val: u32) -> u32 {
    let uimm6_in_0: u32 = ((val) & (0x3fu32));
    let uimm6_out_0: u32 = (0x4u32).wrapping_add(((((0x0u32) << (0x6u32))) | (uimm6_in_0)));
    val = uimm6_out_0;
    val
}

fn sem_ai4const(mut val: u32) -> u32 {
    let ai4const_in_0: u32 = ((val) & (0xfu32));
    let ai4const_out_0: u32 = CONST_TBL_AI4C_0[(((ai4const_in_0) & (0xfu32))) as usize];
    val = ai4const_out_0;
    val
}

fn sem_b4const(mut val: u32) -> u32 {
    let b4const_in_0: u32 = ((val) & (0xfu32));
    let b4const_out_0: u32 = CONST_TBL_B4C_0[(((b4const_in_0) & (0xfu32))) as usize];
    val = b4const_out_0;
    val
}

fn sem_b4constu(mut val: u32) -> u32 {
    let b4constu_in_0: u32 = ((val) & (0xfu32));
    let b4constu_out_0: u32 = CONST_TBL_B4CU_0[(((b4constu_in_0) & (0xfu32))) as usize];
    val = b4constu_out_0;
    val
}

fn sem_uimm8(mut val: u32) -> u32 {
    let uimm8_in_0: u32 = ((val) & (0xffu32));
    let uimm8_out_0: u32 = uimm8_in_0;
    val = uimm8_out_0;
    val
}

fn sem_uimm8x2(mut val: u32) -> u32 {
    let uimm8x2_in_0: u32 = ((val) & (0xffu32));
    let uimm8x2_out_0: u32 = ((uimm8x2_in_0) << (0x1u32));
    val = uimm8x2_out_0;
    val
}

fn sem_uimm8x4(mut val: u32) -> u32 {
    let uimm8x4_in_0: u32 = ((val) & (0xffu32));
    let uimm8x4_out_0: u32 = ((uimm8x4_in_0) << (0x2u32));
    val = uimm8x4_out_0;
    val
}

fn sem_uimm4x16(mut val: u32) -> u32 {
    let uimm4x16_in_0: u32 = ((val) & (0xfu32));
    let uimm4x16_out_0: u32 = ((uimm4x16_in_0) << (0x4u32));
    val = uimm4x16_out_0;
    val
}

fn sem_uimmrx4(mut val: u32) -> u32 {
    let uimmrx4_in_0: u32 = ((val) & (0xfu32));
    let uimmrx4_out_0: u32 = ((uimmrx4_in_0) << (0x2u32));
    val = uimmrx4_out_0;
    val
}

fn sem_simm8(mut val: u32) -> u32 {
    let simm8_in_0: u32 = ((val) & (0xffu32));
    let simm8_out_0: u32 = ((((((simm8_in_0) << (0x18u32))) as i32) >> (0x18u32)) as u32);
    val = simm8_out_0;
    val
}

fn sem_simm8x256(mut val: u32) -> u32 {
    let simm8x256_in_0: u32 = ((val) & (0xffu32));
    let simm8x256_out_0: u32 = ((((((((simm8x256_in_0) << (0x18u32))) as i32) >> (0x18u32)) as u32)) << (0x8u32));
    val = simm8x256_out_0;
    val
}

fn sem_simm12b(mut val: u32) -> u32 {
    let simm12b_in_0: u32 = ((val) & (0xfffu32));
    let simm12b_out_0: u32 = ((((((simm12b_in_0) << (0x14u32))) as i32) >> (0x14u32)) as u32);
    val = simm12b_out_0;
    val
}

fn sem_msalp32(mut val: u32) -> u32 {
    let msalp32_in_0: u32 = ((val) & (0x1fu32));
    let msalp32_out_0: u32 = (0x20u32).wrapping_sub(msalp32_in_0);
    val = msalp32_out_0;
    val
}

fn sem_op2p1(mut val: u32) -> u32 {
    let op2p1_in_0: u32 = ((val) & (0xfu32));
    let op2p1_out_0: u32 = (op2p1_in_0).wrapping_add(0x1u32);
    val = op2p1_out_0;
    val
}

fn sem_label8(mut val: u32) -> u32 {
    let label8_in_0: u32 = ((val) & (0xffu32));
    let label8_out_0: u32 = (0x4u32).wrapping_add(((((((label8_in_0) << (0x18u32))) as i32) >> (0x18u32)) as u32));
    val = label8_out_0;
    val
}

fn sem_ulabel8(mut val: u32) -> u32 {
    let ulabel8_in_0: u32 = ((val) & (0xffu32));
    let ulabel8_out_0: u32 = (0x4u32).wrapping_add(((((0x0u32) << (0x8u32))) | (ulabel8_in_0)));
    val = ulabel8_out_0;
    val
}

fn sem_label12(mut val: u32) -> u32 {
    let label12_in_0: u32 = ((val) & (0xfffu32));
    let label12_out_0: u32 = (0x4u32).wrapping_add(((((((label12_in_0) << (0x14u32))) as i32) >> (0x14u32)) as u32));
    val = label12_out_0;
    val
}

fn sem_soffset(mut val: u32) -> u32 {
    let soffset_in_0: u32 = ((val) & (0x3ffffu32));
    let soffset_out_0: u32 = (0x4u32).wrapping_add(((((((soffset_in_0) << (0xeu32))) as i32) >> (0xeu32)) as u32));
    val = soffset_out_0;
    val
}

fn sem_uimm16x4(mut val: u32) -> u32 {
    let uimm16x4_in_0: u32 = ((val) & (0xffffu32));
    let uimm16x4_out_0: u32 = ((((((0xffffu32) << (0x10u32))) | (uimm16x4_in_0))) << (0x2u32));
    val = uimm16x4_out_0;
    val
}

fn sem_bbi(mut val: u32) -> u32 {
    let bbi_in_0: u32 = ((val) & (0x1fu32));
    let bbi_out_0: u32 = ((((0x0u32) << (0x5u32))) | (bbi_in_0));
    val = bbi_out_0;
    val
}

fn sem_s(mut val: u32) -> u32 {
    let s_in_0: u32 = ((val) & (0xfu32));
    let s_out_0: u32 = ((((0x0u32) << (0x4u32))) | (s_in_0));
    val = s_out_0;
    val
}

fn sem_mr(val: u32) -> u32 {
    val
}

fn sem_mr_1(val: u32) -> u32 {
    val
}

fn sem_mr_2(val: u32) -> u32 {
    val
}

fn sem_mr_3(val: u32) -> u32 {
    val
}

fn sem_mr_4(val: u32) -> u32 {
    val
}

fn sem_mr_5(val: u32) -> u32 {
    val
}

fn sem_immt(mut val: u32) -> u32 {
    let immt_in_0: u32 = ((val) & (0xfu32));
    let immt_out_0: u32 = immt_in_0;
    val = immt_out_0;
    val
}

fn sem_br(val: u32) -> u32 {
    val
}

fn sem_br2(mut val: u32) -> u32 {
    val = ((val) << (0x1u32));
    val
}

fn sem_br4(mut val: u32) -> u32 {
    val = ((val) << (0x2u32));
    val
}

fn sem_br8(mut val: u32) -> u32 {
    val = ((val) << (0x3u32));
    val
}

fn sem_br16(mut val: u32) -> u32 {
    val = ((val) << (0x4u32));
    val
}

fn sem_tp7(mut val: u32) -> u32 {
    let tp7_in_0: u32 = ((val) & (0xfu32));
    let tp7_out_0: u32 = (tp7_in_0).wrapping_add(0x7u32);
    val = tp7_out_0;
    val
}

fn sem_xt_wbr15_label(mut val: u32) -> u32 {
    let xt_wbr15_label_in_0: u32 = ((val) & (0x7fffu32));
    let xt_wbr15_label_out_0: u32 = (0x4u32).wrapping_add(((((((xt_wbr15_label_in_0) << (0x11u32))) as i32) >> (0x11u32)) as u32));
    val = xt_wbr15_label_out_0;
    val
}

fn sem_xt_wbr18_label(mut val: u32) -> u32 {
    let xt_wbr18_label_in_0: u32 = ((val) & (0x3ffffu32));
    let xt_wbr18_label_out_0: u32 = (0x4u32).wrapping_add(((((((xt_wbr18_label_in_0) << (0xeu32))) as i32) >> (0xeu32)) as u32));
    val = xt_wbr18_label_out_0;
    val
}

fn sem_fr(val: u32) -> u32 {
    val
}

fn sem_imm_t(mut val: u32) -> u32 {
    let imm_t_in_0: u32 = ((val) & (0xfu32));
    let imm_t_out_0: u32 = ((((0x0u32) << (0x4u32))) | (imm_t_in_0));
    val = imm_t_out_0;
    val
}

fn sem_imm8x4(mut val: u32) -> u32 {
    let imm8x4_in_0: u32 = ((val) & (0xffu32));
    let imm8x4_out_0: u32 = ((((((0x0u32) << (0xau32))) | (((imm8x4_in_0) << (0x2u32))))) | (0x0u32));
    val = imm8x4_out_0;
    val
}

fn sem_imm8x8(mut val: u32) -> u32 {
    let imm8x8_in_0: u32 = ((val) & (0xffu32));
    let imm8x8_out_0: u32 = ((((((0x0u32) << (0xbu32))) | (((imm8x8_in_0) << (0x3u32))))) | (0x0u32));
    val = imm8x8_out_0;
    val
}

fn sem_qr(val: u32) -> u32 {
    val
}

fn sem_opnd_semantic_ld_st_imm16f(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_imm16f_in_0: u32 = ((val) & (0xfu32));
    let opnd_semantic_LD_ST_imm16f_out_0: u32 = ((((((((opnd_semantic_LD_ST_imm16f_in_0) << (0x1cu32))) as i32) >> (0x1cu32)) as u32)) << (0x4u32));
    val = opnd_semantic_LD_ST_imm16f_out_0;
    val
}

fn sem_opnd_ee_set_bit_gpio_out_mask(mut val: u32) -> u32 {
    let opnd_EE_SET_BIT_GPIO_OUT_mask_in_0: u32 = ((val) & (0xffu32));
    let opnd_EE_SET_BIT_GPIO_OUT_mask_out_0: u32 = ((((0x0u32) << (0x8u32))) | (opnd_EE_SET_BIT_GPIO_OUT_mask_in_0));
    val = opnd_EE_SET_BIT_GPIO_OUT_mask_out_0;
    val
}

fn sem_opnd_semantic_ld_st_imm2(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_imm2_in_0: u32 = ((val) & (0x7fu32));
    let opnd_semantic_LD_ST_imm2_out_0: u32 = ((((((0x0u32) << (0x7u32))) | (opnd_semantic_LD_ST_imm2_in_0))) << (0x1u32));
    val = opnd_semantic_LD_ST_imm2_out_0;
    val
}

fn sem_opnd_semantic_ld_st_imm1(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_imm1_in_0: u32 = ((val) & (0x7fu32));
    let opnd_semantic_LD_ST_imm1_out_0: u32 = ((((0x0u32) << (0x7u32))) | (opnd_semantic_LD_ST_imm1_in_0));
    val = opnd_semantic_LD_ST_imm1_out_0;
    val
}

fn sem_opnd_semantic_ld_st_imm8(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_imm8_in_0: u32 = ((val) & (0xffu32));
    let opnd_semantic_LD_ST_imm8_out_0: u32 = ((((((((opnd_semantic_LD_ST_imm8_in_0) << (0x18u32))) as i32) >> (0x18u32)) as u32)) << (0x3u32));
    val = opnd_semantic_LD_ST_imm8_out_0;
    val
}

fn sem_opnd_semantic_ld_st_imm16(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_imm16_in_0: u32 = ((val) & (0xffu32));
    let opnd_semantic_LD_ST_imm16_out_0: u32 = ((((((((opnd_semantic_LD_ST_imm16_in_0) << (0x18u32))) as i32) >> (0x18u32)) as u32)) << (0x4u32));
    val = opnd_semantic_LD_ST_imm16_out_0;
    val
}

fn sem_opnd_semantic_ld_st_imm4(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_imm4_in_0: u32 = ((val) & (0xffu32));
    let opnd_semantic_LD_ST_imm4_out_0: u32 = ((((((((opnd_semantic_LD_ST_imm4_in_0) << (0x18u32))) as i32) >> (0x18u32)) as u32)) << (0x2u32));
    val = opnd_semantic_LD_ST_imm4_out_0;
    val
}

fn sem_opnd_semantic_ld_st_sel8(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_sel8_in_0: u32 = ((val) & (0x7u32));
    let opnd_semantic_LD_ST_sel8_out_0: u32 = ((((0x0u32) << (0x3u32))) | (opnd_semantic_LD_ST_sel8_in_0));
    val = opnd_semantic_LD_ST_sel8_out_0;
    val
}

fn sem_opnd_semantic_ld_st_sel4(mut val: u32) -> u32 {
    let opnd_semantic_LD_ST_sel4_in_0: u32 = ((val) & (0x3u32));
    let opnd_semantic_LD_ST_sel4_out_0: u32 = ((((0x0u32) << (0x2u32))) | (opnd_semantic_LD_ST_sel4_in_0));
    val = opnd_semantic_LD_ST_sel4_out_0;
    val
}

fn sem_opnd_semantic_cal_double_q_imm16(mut val: u32) -> u32 {
    let opnd_semantic_CAL_DOUBLE_Q_imm16_in_0: u32 = ((val) & (0x3fu32));
    let opnd_semantic_CAL_DOUBLE_Q_imm16_out_0: u32 = ((((((((opnd_semantic_CAL_DOUBLE_Q_imm16_in_0) << (0x1au32))) as i32) >> (0x1au32)) as u32)) << (0x4u32));
    val = opnd_semantic_CAL_DOUBLE_Q_imm16_out_0;
    val
}

fn sem_opnd_semantic_cal_double_q_sel16(mut val: u32) -> u32 {
    let opnd_semantic_CAL_DOUBLE_Q_sel16_in_0: u32 = ((val) & (0xfu32));
    let opnd_semantic_CAL_DOUBLE_Q_sel16_out_0: u32 = ((((0x0u32) << (0x4u32))) | (opnd_semantic_CAL_DOUBLE_Q_sel16_in_0));
    val = opnd_semantic_CAL_DOUBLE_Q_sel16_out_0;
    val
}

fn sem_opnd_semantic_srcmb_qacc_sel2(mut val: u32) -> u32 {
    let opnd_semantic_SRCMB_QACC_sel2_in_0: u32 = ((val) & (0x1u32));
    let opnd_semantic_SRCMB_QACC_sel2_out_0: u32 = ((((0x0u32) << (0x1u32))) | (opnd_semantic_SRCMB_QACC_sel2_in_0));
    val = opnd_semantic_SRCMB_QACC_sel2_out_0;
    val
}

fn rtoa_soffsetx4(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(((pc) & ((!(0x3u32)))));
    val
}

fn rtoa_uimm6(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

fn rtoa_label8(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

fn rtoa_ulabel8(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

fn rtoa_label12(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

fn rtoa_soffset(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

fn rtoa_uimm16x4(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add((((pc).wrapping_add(0x3u32)) & ((!(0x3u32)))));
    val
}

fn rtoa_xt_wbr15_label(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

fn rtoa_xt_wbr18_label(mut val: u32, pc: u32) -> u32 {
    val = val.wrapping_add(pc);
    val
}

pub(crate) struct Operand {
    pub field: u16,
    pub sem: fn(u32) -> u32,
    pub rtoa: fn(u32, u32) -> u32,
    pub prefix: u8,
    pub width: u8,
    pub pcrel: bool,
    pub visible: bool,
}

pub(crate) static OPERANDS: [Operand; 471] = [
    Operand { field: 10, sem: sem_soffsetx4, rtoa: rtoa_soffsetx4, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 3, sem: sem_uimm12x8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 26, sem: sem_simm4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: false },
    Operand { field: 0, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 251, sem: sem_ar_0, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: false },
    Operand { field: 252, sem: sem_ar_1, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: false },
    Operand { field: 253, sem: sem_ar_2, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: false },
    Operand { field: 254, sem: sem_ar_3, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: false },
    Operand { field: 5, sem: sem_ar_4, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_immrx4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_lsi4x4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 34, sem: sem_simm7, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 33, sem: sem_uimm6, rtoa: rtoa_uimm6, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 0, sem: sem_ai4const, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_b4const, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_b4constu, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_uimm8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_uimm8x2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_uimm8x4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 13, sem: sem_uimm4x16, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_uimmrx4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_simm8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_simm8x256, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 6, sem: sem_simm12b, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 18, sem: sem_msalp32, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 13, sem: sem_op2p1, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_label8, rtoa: rtoa_label8, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 4, sem: sem_ulabel8, rtoa: rtoa_ulabel8, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 3, sem: sem_label12, rtoa: rtoa_label12, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 10, sem: sem_soffset, rtoa: rtoa_soffset, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 7, sem: sem_uimm16x4, rtoa: rtoa_uimm16x4, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 2, sem: sem_bbi, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 17, sem: sem_bbi, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 21, sem: sem_bbi, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 19, sem: sem_bbi, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_s, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 43, sem: sem_mr, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: true },
    Operand { field: 42, sem: sem_mr_0, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: true },
    Operand { field: 41, sem: sem_mr_1, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: true },
    Operand { field: 255, sem: sem_mr_2, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: false },
    Operand { field: 256, sem: sem_mr_3, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: false },
    Operand { field: 257, sem: sem_mr_4, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: false },
    Operand { field: 258, sem: sem_mr_5, rtoa: rtoa_id, prefix: b'm', width: 1, pcrel: false, visible: false },
    Operand { field: 0, sem: sem_immt, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_immt, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 0, sem: sem_br, rtoa: rtoa_id, prefix: b'b', width: 1, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_br, rtoa: rtoa_id, prefix: b'b', width: 1, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_br, rtoa: rtoa_id, prefix: b'b', width: 1, pcrel: false, visible: true },
    Operand { field: 44, sem: sem_br2, rtoa: rtoa_id, prefix: b'b', width: 2, pcrel: false, visible: true },
    Operand { field: 45, sem: sem_br2, rtoa: rtoa_id, prefix: b'b', width: 2, pcrel: false, visible: true },
    Operand { field: 46, sem: sem_br2, rtoa: rtoa_id, prefix: b'b', width: 2, pcrel: false, visible: true },
    Operand { field: 47, sem: sem_br4, rtoa: rtoa_id, prefix: b'b', width: 4, pcrel: false, visible: true },
    Operand { field: 48, sem: sem_br4, rtoa: rtoa_id, prefix: b'b', width: 4, pcrel: false, visible: true },
    Operand { field: 49, sem: sem_br4, rtoa: rtoa_id, prefix: b'b', width: 4, pcrel: false, visible: true },
    Operand { field: 50, sem: sem_br8, rtoa: rtoa_id, prefix: b'b', width: 8, pcrel: false, visible: true },
    Operand { field: 51, sem: sem_br8, rtoa: rtoa_id, prefix: b'b', width: 8, pcrel: false, visible: true },
    Operand { field: 52, sem: sem_br8, rtoa: rtoa_id, prefix: b'b', width: 8, pcrel: false, visible: true },
    Operand { field: 259, sem: sem_br16, rtoa: rtoa_id, prefix: b'b', width: 16, pcrel: false, visible: true },
    Operand { field: 260, sem: sem_br16, rtoa: rtoa_id, prefix: b'b', width: 16, pcrel: false, visible: true },
    Operand { field: 261, sem: sem_br16, rtoa: rtoa_id, prefix: b'b', width: 16, pcrel: false, visible: true },
    Operand { field: 262, sem: sem_br16, rtoa: rtoa_id, prefix: b'b', width: 16, pcrel: false, visible: false },
    Operand { field: 0, sem: sem_tp7, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 53, sem: sem_xt_wbr15_label, rtoa: rtoa_xt_wbr15_label, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 54, sem: sem_xt_wbr18_label, rtoa: rtoa_xt_wbr18_label, prefix: 0, width: 0, pcrel: true, visible: true },
    Operand { field: 14, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 0, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 0, sem: sem_imm_t, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 5, sem: sem_imm_t, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_imm8x4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_imm8x8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 57, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 58, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 59, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 60, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 61, sem: sem_opnd_semantic_ld_st_imm16f, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 64, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 65, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 66, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 67, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 68, sem: sem_opnd_ee_set_bit_gpio_out_mask, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 69, sem: sem_opnd_ee_set_bit_gpio_out_mask, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 71, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 73, sem: sem_opnd_semantic_ld_st_imm2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 75, sem: sem_opnd_semantic_ld_st_imm1, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 77, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 78, sem: sem_opnd_semantic_ld_st_imm16f, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 79, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 80, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 81, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 82, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 83, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 85, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 86, sem: sem_opnd_semantic_ld_st_imm8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 93, sem: sem_opnd_semantic_ld_st_imm16, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 95, sem: sem_opnd_semantic_ld_st_imm4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 96, sem: sem_opnd_semantic_ld_st_sel8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 97, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 98, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 99, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 101, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 102, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 103, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 104, sem: sem_fr, rtoa: rtoa_id, prefix: b'f', width: 1, pcrel: false, visible: true },
    Operand { field: 105, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 106, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 107, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 108, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 109, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 110, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 111, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 112, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 113, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 114, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 115, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 116, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 117, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 119, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 120, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 121, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 122, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 123, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 124, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 126, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 128, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 129, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 130, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 131, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 134, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 135, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 136, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 138, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 141, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 142, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 143, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 144, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 145, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 146, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 148, sem: sem_opnd_semantic_cal_double_q_imm16, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 152, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 155, sem: sem_opnd_semantic_ld_st_sel8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 158, sem: sem_opnd_semantic_cal_double_q_sel16, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 160, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 161, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 162, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 165, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 166, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 167, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 168, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 169, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 171, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 173, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 174, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 175, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 177, sem: sem_opnd_semantic_cal_double_q_sel16, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 178, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 179, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 180, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 181, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 182, sem: sem_opnd_semantic_ld_st_imm16, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 183, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 187, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 189, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 190, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 191, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 194, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 195, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 196, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 197, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 198, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 199, sem: sem_opnd_semantic_ld_st_sel8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 200, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 201, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 202, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 203, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 204, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 205, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 206, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 207, sem: sem_opnd_semantic_ld_st_sel4, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 208, sem: sem_opnd_semantic_ld_st_sel8, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 209, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 210, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 211, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 212, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 213, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 214, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 215, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 216, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 217, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 218, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 219, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 220, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 221, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 222, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 223, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 224, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 225, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 226, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 227, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 228, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 229, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 230, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 231, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 232, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 233, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 234, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 235, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 236, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 237, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 238, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 239, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 240, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 241, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 242, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 243, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 244, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 245, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 246, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 247, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 248, sem: sem_opnd_semantic_srcmb_qacc_sel2, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 249, sem: sem_ar, rtoa: rtoa_id, prefix: b'a', width: 1, pcrel: false, visible: true },
    Operand { field: 250, sem: sem_qr, rtoa: rtoa_id, prefix: b'q', width: 1, pcrel: false, visible: true },
    Operand { field: 0, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 1, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 3, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 4, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 6, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 7, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 8, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 9, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 10, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 11, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 12, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 13, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 14, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 15, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 16, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 18, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 20, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 22, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 23, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 24, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 25, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 26, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 27, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 28, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 29, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 30, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 31, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 32, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 33, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 34, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 35, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 36, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 37, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 38, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 39, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 40, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 41, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 42, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 43, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 44, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 45, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 46, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 47, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 48, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 49, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 50, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 51, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 52, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 53, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 54, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 55, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 56, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 57, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 58, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 59, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 60, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 61, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 62, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 63, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 64, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 65, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 66, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 67, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 68, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 69, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 70, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 71, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 72, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 73, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 74, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 75, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 76, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 77, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 78, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 79, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 80, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 81, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 82, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 83, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 84, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 85, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 86, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 87, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 88, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 89, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 90, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 91, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 92, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 93, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 94, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 95, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 96, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 97, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 98, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 99, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 100, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 101, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 102, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 103, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 104, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 105, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 106, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 107, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 108, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 109, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 110, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 111, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 112, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 113, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 114, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 115, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 116, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 117, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 118, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 119, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 120, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 121, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 122, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 123, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 124, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 125, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 126, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 127, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 128, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 129, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 130, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 131, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 132, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 133, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 134, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 135, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 136, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 137, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 138, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 139, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 140, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 141, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 142, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 143, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 144, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 145, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 146, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 147, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 148, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 149, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 150, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 151, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 152, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 153, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 154, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 155, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 156, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 157, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 158, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 159, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 160, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 161, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 162, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 163, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 164, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 165, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 166, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 167, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 168, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 169, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 170, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 171, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 172, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 173, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 174, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 175, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 176, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 177, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 178, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 179, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 180, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 181, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 182, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 183, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 184, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 185, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 186, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 187, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 188, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 189, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 190, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 191, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 192, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 193, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 194, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 195, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 196, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 197, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 198, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 199, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 200, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 201, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 202, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 203, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 204, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 205, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 206, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 207, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 208, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 209, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 210, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 211, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 212, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 213, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 214, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 215, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 216, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 217, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 218, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 219, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 220, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 221, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 222, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 223, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 224, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 225, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 226, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 227, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 228, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 229, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 230, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 231, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 232, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 233, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 234, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 235, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 236, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 237, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 238, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 239, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 240, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 241, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 242, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 243, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 244, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 245, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 246, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 247, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 248, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 249, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
    Operand { field: 250, sem: sem_id, rtoa: rtoa_id, prefix: 0, width: 0, pcrel: false, visible: true },
];

pub(crate) struct ArgUse {
    pub operand: u16,
    pub show: bool,
}

pub(crate) static ICLASS_OPS: [&[ArgUse]; 630] = [
    &[ArgUse { operand: 69, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 72, show: true }],
    &[ArgUse { operand: 69, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 72, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 69, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 72, show: true }],
    &[ArgUse { operand: 69, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 72, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 48, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 48, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 70, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }, ArgUse { operand: 69, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 71, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[ArgUse { operand: 67, show: true }, ArgUse { operand: 68, show: true }],
    &[],
    &[],
    &[],
    &[],
    &[ArgUse { operand: 0, show: true }, ArgUse { operand: 10, show: true }],
    &[ArgUse { operand: 0, show: true }, ArgUse { operand: 9, show: true }],
    &[ArgUse { operand: 0, show: true }, ArgUse { operand: 8, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 10, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 9, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 8, show: true }],
    &[ArgUse { operand: 11, show: false }, ArgUse { operand: 4, show: true }, ArgUse { operand: 1, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 2, show: true }],
    &[ArgUse { operand: 5, show: true }],
    &[],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 12, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 12, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 16, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 15, show: true }],
    &[],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 13, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 14, show: true }],
    &[],
    &[ArgUse { operand: 5, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 13, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 24, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 25, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 17, show: true }, ArgUse { operand: 29, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 34, show: true }, ArgUse { operand: 29, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 18, show: true }, ArgUse { operand: 29, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }, ArgUse { operand: 29, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 31, show: true }],
    &[ArgUse { operand: 0, show: true }, ArgUse { operand: 7, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 7, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 6, show: true }, ArgUse { operand: 35, show: true }, ArgUse { operand: 28, show: true }],
    &[],
    &[ArgUse { operand: 32, show: true }],
    &[ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 20, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 20, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 21, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 33, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 19, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 30, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 30, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 26, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 6, show: true }],
    &[],
    &[ArgUse { operand: 5, show: true }],
    &[],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 20, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 21, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 23, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 19, show: true }],
    &[ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 36, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 27, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 6, show: true }, ArgUse { operand: 37, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 6, show: true }, ArgUse { operand: 38, show: true }],
    &[],
    &[],
    &[],
    &[],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 38, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 40, show: true }],
    &[ArgUse { operand: 39, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 39, show: true }, ArgUse { operand: 40, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 4, show: true }, ArgUse { operand: 40, show: true }],
    &[ArgUse { operand: 39, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 39, show: true }, ArgUse { operand: 40, show: true }],
    &[ArgUse { operand: 41, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 39, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 41, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 39, show: true }, ArgUse { operand: 40, show: true }],
    &[ArgUse { operand: 41, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 42, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 42, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 42, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 43, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 43, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 43, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 44, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 44, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 44, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 45, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 45, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 45, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 38, show: true }],
    &[ArgUse { operand: 38, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 47, show: true }, ArgUse { operand: 46, show: true }],
    &[ArgUse { operand: 47, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 47, show: true }],
    &[],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 50, show: true }, ArgUse { operand: 49, show: true }, ArgUse { operand: 48, show: true }],
    &[ArgUse { operand: 48, show: true }, ArgUse { operand: 55, show: true }],
    &[ArgUse { operand: 48, show: true }, ArgUse { operand: 58, show: true }],
    &[ArgUse { operand: 49, show: true }, ArgUse { operand: 29, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 48, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 63, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 63, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 63, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 64, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 64, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 21, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 21, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 21, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }, ArgUse { operand: 4, show: true }, ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 6, show: true }, ArgUse { operand: 4, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 75, show: true }, ArgUse { operand: 74, show: true }, ArgUse { operand: 78, show: true }],
    &[ArgUse { operand: 77, show: true }, ArgUse { operand: 76, show: true }, ArgUse { operand: 78, show: true }],
    &[ArgUse { operand: 80, show: true }, ArgUse { operand: 79, show: true }],
    &[ArgUse { operand: 82, show: true }, ArgUse { operand: 81, show: true }],
    &[ArgUse { operand: 83, show: true }],
    &[ArgUse { operand: 84, show: true }],
    &[ArgUse { operand: 85, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 86, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 98, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 87, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 88, show: true }, ArgUse { operand: 90, show: true }],
    &[ArgUse { operand: 94, show: true }, ArgUse { operand: 93, show: true }, ArgUse { operand: 92, show: true }, ArgUse { operand: 91, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 89, show: true }],
    &[ArgUse { operand: 94, show: true }, ArgUse { operand: 93, show: true }, ArgUse { operand: 92, show: true }, ArgUse { operand: 91, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 92, show: true }, ArgUse { operand: 91, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 92, show: true }, ArgUse { operand: 91, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 98, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 98, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 102, show: true }, ArgUse { operand: 101, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 100, show: true }, ArgUse { operand: 99, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 106, show: true }, ArgUse { operand: 105, show: true }, ArgUse { operand: 104, show: true }, ArgUse { operand: 103, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 89, show: true }],
    &[ArgUse { operand: 106, show: true }, ArgUse { operand: 105, show: true }, ArgUse { operand: 104, show: true }, ArgUse { operand: 103, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 104, show: true }, ArgUse { operand: 103, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 104, show: true }, ArgUse { operand: 103, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 95, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 98, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 98, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 97, show: true }],
    &[ArgUse { operand: 90, show: true }, ArgUse { operand: 96, show: true }],
    &[ArgUse { operand: 107, show: true }, ArgUse { operand: 101, show: true }, ArgUse { operand: 90, show: true }, ArgUse { operand: 100, show: true }, ArgUse { operand: 99, show: true }],
    &[ArgUse { operand: 110, show: true }, ArgUse { operand: 109, show: true }, ArgUse { operand: 108, show: true }],
    &[ArgUse { operand: 113, show: true }, ArgUse { operand: 112, show: true }, ArgUse { operand: 111, show: true }],
    &[ArgUse { operand: 115, show: true }, ArgUse { operand: 114, show: true }],
    &[ArgUse { operand: 117, show: true }, ArgUse { operand: 116, show: true }],
    &[ArgUse { operand: 119, show: true }, ArgUse { operand: 118, show: true }],
    &[ArgUse { operand: 121, show: true }, ArgUse { operand: 120, show: true }],
    &[ArgUse { operand: 123, show: true }, ArgUse { operand: 122, show: true }],
    &[ArgUse { operand: 125, show: true }, ArgUse { operand: 124, show: true }],
    &[],
    &[],
    &[ArgUse { operand: 126, show: true }],
    &[ArgUse { operand: 127, show: true }],
    &[ArgUse { operand: 128, show: true }],
    &[ArgUse { operand: 129, show: true }],
    &[ArgUse { operand: 130, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 135, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 135, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 135, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 141, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 140, show: true }, ArgUse { operand: 139, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 142, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 143, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 143, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 144, show: true }],
    &[ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 144, show: true }],
    &[ArgUse { operand: 147, show: true }, ArgUse { operand: 146, show: true }, ArgUse { operand: 145, show: true }],
    &[ArgUse { operand: 147, show: true }, ArgUse { operand: 146, show: true }, ArgUse { operand: 145, show: true }],
    &[ArgUse { operand: 150, show: true }, ArgUse { operand: 149, show: true }, ArgUse { operand: 148, show: true }],
    &[ArgUse { operand: 152, show: true }, ArgUse { operand: 151, show: true }, ArgUse { operand: 153, show: true }],
    &[ArgUse { operand: 152, show: true }, ArgUse { operand: 151, show: true }, ArgUse { operand: 153, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 153, show: true }],
    &[ArgUse { operand: 138, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }, ArgUse { operand: 153, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 133, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 134, show: true }, ArgUse { operand: 132, show: true }, ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 131, show: true }, ArgUse { operand: 137, show: true }, ArgUse { operand: 136, show: true }],
    &[ArgUse { operand: 156, show: true }, ArgUse { operand: 155, show: true }, ArgUse { operand: 154, show: true }],
    &[ArgUse { operand: 156, show: true }, ArgUse { operand: 155, show: true }],
    &[ArgUse { operand: 156, show: true }, ArgUse { operand: 155, show: true }, ArgUse { operand: 154, show: true }],
    &[ArgUse { operand: 156, show: true }, ArgUse { operand: 155, show: true }, ArgUse { operand: 154, show: true }],
    &[ArgUse { operand: 159, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 157, show: true }],
    &[ArgUse { operand: 159, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 161, show: true }, ArgUse { operand: 160, show: true }],
    &[ArgUse { operand: 159, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 157, show: true }],
    &[ArgUse { operand: 158, show: true }, ArgUse { operand: 159, show: true }, ArgUse { operand: 161, show: true }],
    &[ArgUse { operand: 163, show: true }, ArgUse { operand: 161, show: true }, ArgUse { operand: 162, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 159, show: true }],
    &[ArgUse { operand: 163, show: true }, ArgUse { operand: 161, show: true }, ArgUse { operand: 160, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 159, show: true }],
    &[ArgUse { operand: 164, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 159, show: true }],
    &[ArgUse { operand: 164, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 159, show: true }],
    &[ArgUse { operand: 159, show: true }, ArgUse { operand: 158, show: true }, ArgUse { operand: 161, show: true }, ArgUse { operand: 160, show: true }],
    &[ArgUse { operand: 164, show: true }, ArgUse { operand: 165, show: true }],
    &[ArgUse { operand: 164, show: true }, ArgUse { operand: 165, show: true }],
    &[ArgUse { operand: 172, show: true }, ArgUse { operand: 170, show: true }, ArgUse { operand: 169, show: true }, ArgUse { operand: 167, show: true }, ArgUse { operand: 166, show: true }],
    &[ArgUse { operand: 172, show: true }, ArgUse { operand: 171, show: true }, ArgUse { operand: 170, show: true }, ArgUse { operand: 169, show: true }, ArgUse { operand: 168, show: true }],
    &[ArgUse { operand: 179, show: true }, ArgUse { operand: 178, show: true }, ArgUse { operand: 177, show: true }, ArgUse { operand: 176, show: true }, ArgUse { operand: 175, show: true }, ArgUse { operand: 174, show: true }, ArgUse { operand: 173, show: true }],
    &[ArgUse { operand: 187, show: true }, ArgUse { operand: 186, show: true }, ArgUse { operand: 185, show: true }, ArgUse { operand: 184, show: true }, ArgUse { operand: 183, show: true }, ArgUse { operand: 182, show: true }, ArgUse { operand: 181, show: true }, ArgUse { operand: 180, show: true }],
    &[ArgUse { operand: 189, show: true }, ArgUse { operand: 188, show: true }],
    &[ArgUse { operand: 197, show: true }, ArgUse { operand: 196, show: true }, ArgUse { operand: 195, show: true }, ArgUse { operand: 194, show: true }, ArgUse { operand: 193, show: true }, ArgUse { operand: 192, show: true }, ArgUse { operand: 191, show: true }, ArgUse { operand: 190, show: true }],
    &[ArgUse { operand: 205, show: true }, ArgUse { operand: 204, show: true }, ArgUse { operand: 203, show: true }, ArgUse { operand: 202, show: true }, ArgUse { operand: 201, show: true }, ArgUse { operand: 200, show: true }, ArgUse { operand: 199, show: true }, ArgUse { operand: 198, show: true }],
    &[ArgUse { operand: 213, show: true }, ArgUse { operand: 212, show: true }, ArgUse { operand: 211, show: true }, ArgUse { operand: 210, show: true }, ArgUse { operand: 209, show: true }, ArgUse { operand: 208, show: true }, ArgUse { operand: 207, show: true }, ArgUse { operand: 206, show: true }],
    &[ArgUse { operand: 221, show: true }, ArgUse { operand: 220, show: true }, ArgUse { operand: 219, show: true }, ArgUse { operand: 218, show: true }, ArgUse { operand: 217, show: true }, ArgUse { operand: 216, show: true }, ArgUse { operand: 215, show: true }, ArgUse { operand: 214, show: true }],
    &[ArgUse { operand: 224, show: true }, ArgUse { operand: 223, show: true }, ArgUse { operand: 222, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
    &[ArgUse { operand: 3, show: true }],
    &[ArgUse { operand: 6, show: true }],
];

fn field_r_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_op0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_op2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 8) >> 28);
    tie
}

fn field_op1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 12) >> 28);
    tie
}

fn field_t_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_s_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_n_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_m_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie
}

fn field_sr_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_st_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_thi3_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 24) >> 29);
    tie
}

fn field_t3_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_tlo_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_w_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 18) >> 30);
    tie
}

fn field_r3_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_rhi_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 16) >> 30);
    tie
}

fn field_fld_inst_23_22_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 8) >> 30);
    tie
}

fn field_fld_inst_19_16_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 12) >> 28);
    tie
}

fn field_fld_inst_14_12_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_inst_3_0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_inst_9_6_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 22) >> 28);
    tie
}

fn field_fld_inst_4_0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 5) | ((insn << 27) >> 27);
    tie
}

fn field_fld_inst_23_12_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 12) | ((insn << 8) >> 20);
    tie
}

fn field_fld_inst_23_8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 16) | ((insn << 8) >> 16);
    tie
}

fn field_fld_inst_23_23_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 8) >> 31);
    tie
}

fn field_fld_inst_14_8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 17) >> 25);
    tie
}

fn field_fld_inst_11_8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_inst_21_15_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 10) >> 25);
    tie
}

fn field_fld_inst_9_8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 22) >> 30);
    tie
}

fn field_fld_inst_11_0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 12) | ((insn << 20) >> 20);
    tie
}

fn field_fld_inst_23_0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 24) | ((insn << 8) >> 8);
    tie
}

fn field_fld_inst_14_0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 15) | ((insn << 17) >> 17);
    tie
}

fn field_fld_inst_13_13_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 18) >> 31);
    tie
}

fn field_fld_inst_7_0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 8) | ((insn << 24) >> 24);
    tie
}

fn field_fld_inst_7_6_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie
}

fn field_fld_inst_23_15_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 9) | ((insn << 8) >> 23);
    tie
}

fn field_fld_inst_23_21_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 8) >> 29);
    tie
}

fn field_fld_inst_7_5_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 24) >> 29);
    tie
}

fn field_fld_inst_14_11_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 17) >> 28);
    tie
}

fn field_fld_inst_13_12_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 18) >> 30);
    tie
}

fn field_fld_inst_5_5_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 26) >> 31);
    tie
}

fn field_fld_inst_11_7_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 5) | ((insn << 20) >> 27);
    tie
}

fn field_fld_inst_14_7_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 8) | ((insn << 17) >> 24);
    tie
}

fn field_fld_inst_9_9_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 22) >> 31);
    tie
}

fn field_op0_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_t_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_r_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_op0_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_z_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 25) >> 31);
    tie
}

fn field_i_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_s_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_slot_format_32_0_28_4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 25) | ((insn << 3) >> 7);
    tie
}

fn field_fld_slot_format_32_0_3_0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 5) | ((insn << 3) >> 27);
    tie
}

fn field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 6) | ((insn << 3) >> 26);
    tie
}

fn field_fld_slot_format_32_0_7_5_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 24) >> 29);
    tie
}

fn field_fld_slot_format_32_0_28_22_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 3) >> 25);
    tie
}

fn field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 21) >> 25);
    tie
}

fn field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 9) | ((insn << 3) >> 23);
    tie
}

fn field_fld_slot_format_32_0_10_6_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 5) | ((insn << 21) >> 27);
    tie
}

fn field_fld_slot_format_32_0_7_6_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie
}

fn field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 3) >> 28);
    tie
}

fn field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_slot_format_32_0_10_9_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 21) >> 30);
    tie
}

fn field_fld_slot_format_32_0_13_11_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_slot_format_32_0_13_13_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 18) >> 31);
    tie
}

fn field_t_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_bbi4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 19) >> 31);
    tie
}

fn field_bbi_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 19) >> 31);
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_imm12_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 12) | ((insn << 8) >> 20);
    tie
}

fn field_imm8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 8) | ((insn << 8) >> 24);
    tie
}

fn field_s_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_imm12b_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie = (tie << 8) | ((insn << 8) >> 24);
    tie
}

fn field_imm16_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 16) | ((insn << 8) >> 16);
    tie
}

fn field_offset_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 18) | ((insn << 8) >> 14);
    tie
}

fn field_r_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_sa4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 11) >> 31);
    tie
}

fn field_sae4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 15) >> 31);
    tie
}

fn field_sae_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 15) >> 31);
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_sal_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 11) >> 31);
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_sargt_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 11) >> 31);
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_sas4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 27) >> 31);
    tie
}

fn field_sas_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 27) >> 31);
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_sr_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_sr_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_st_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_st_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_imm4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm4_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm4_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_mn_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_i_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_imm6lo_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm6lo_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm6hi_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_imm6hi_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_imm7lo_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm7lo_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm7hi_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie
}

fn field_imm7hi_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie
}

fn field_z_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 25) >> 31);
    tie
}

fn field_imm6_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm6_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm7_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_imm7_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_rbit2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 17) >> 31);
    tie
}

fn field_tbit2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 25) >> 31);
    tie
}

fn field_y_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 25) >> 31);
    tie
}

fn field_x_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 17) >> 31);
    tie
}

fn field_t2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 24) >> 29);
    tie
}

fn field_t2_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 24) >> 29);
    tie
}

fn field_t2_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 24) >> 29);
    tie
}

fn field_s2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 20) >> 29);
    tie
}

fn field_s2_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 20) >> 29);
    tie
}

fn field_s2_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 20) >> 29);
    tie
}

fn field_r2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 16) >> 29);
    tie
}

fn field_r2_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 16) >> 29);
    tie
}

fn field_r2_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 16) >> 29);
    tie
}

fn field_t4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie
}

fn field_t4_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie
}

fn field_t4_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie
}

fn field_s4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie
}

fn field_s4_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie
}

fn field_s4_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie
}

fn field_r4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 16) >> 30);
    tie
}

fn field_r4_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 16) >> 30);
    tie
}

fn field_r4_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 16) >> 30);
    tie
}

fn field_t8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_t8_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_t8_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_s8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 20) >> 31);
    tie
}

fn field_s8_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 20) >> 31);
    tie
}

fn field_s8_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 20) >> 31);
    tie
}

fn field_r8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_r8_slot_inst16a(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_r8_slot_inst16b(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_xt_wbr15_imm_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 15) | ((insn << 8) >> 17);
    tie
}

fn field_xt_wbr18_imm_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 18) | ((insn << 8) >> 14);
    tie
}

fn field_fld_st_qr_b_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_st_qr_a_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ld_qr_b_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ld_qr_a_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_qr_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_mv_qr_b_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie = (tie << 1) | ((insn << 26) >> 31);
    tie
}

fn field_fld_mv_qr_a_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_wr_mask_gpio_out_mask_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_ee_wr_mask_gpio_out_value_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_set_bit_gpio_out_mask_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 8) | ((insn << 20) >> 24);
    tie
}

fn field_fld_ee_clr_bit_gpio_out_clear_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 8) | ((insn << 20) >> 24);
    tie
}

fn field_fld_ee_get_gpio_in_au_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_ld_st_imm2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 17) >> 25);
    tie
}

fn field_fld_semantic_ld_st_imm1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 17) >> 25);
    tie
}

fn field_fld_semantic_ld_st_qu1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_semantic_ld_st_imm16f_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_ld_st_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_ld_st_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fu0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 8) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fu0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fu1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fu1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fu2_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 12) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fu3_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 8) >> 28);
    tie
}

fn field_fld_semantic_ld_st_ad_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_semantic_ld_st_ad_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_ld_st_imm8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 9) >> 31);
    tie = (tie << 7) | ((insn << 17) >> 25);
    tie
}

fn field_fld_semantic_ld_st_imm8_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 7) | ((insn << 9) >> 25);
    tie = (tie << 1) | ((insn << 27) >> 31);
    tie
}

fn field_fld_semantic_ld_st_imm16_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 9) >> 31);
    tie = (tie << 7) | ((insn << 17) >> 25);
    tie
}

fn field_fld_semantic_ld_st_imm4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 9) >> 31);
    tie = (tie << 7) | ((insn << 17) >> 25);
    tie
}

fn field_fld_semantic_ld_st_sel8_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_semantic_ld_st_sel4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie
}

fn field_fld_semantic_ld_st_qs_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_semantic_ld_st_qu_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_ld_st_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_semantic_ld_st_fv0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 8) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fv0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fv1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fv1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 16) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fv2_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 12) >> 28);
    tie
}

fn field_fld_semantic_ld_st_fv3_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 8) >> 28);
    tie
}

fn field_fld_semantic_ld_st_qv_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_ld_st_qv_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_movi_32_a_sel4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie
}

fn field_fld_ee_movi_32_a_au_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_movi_32_a_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_movi_32_q_sel4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie
}

fn field_fld_ee_movi_32_q_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_movi_32_q_qu_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vzip_16_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vzip_16_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_ee_vzip_32_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vzip_32_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_ee_vzip_8_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vzip_8_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_ee_vunzip_16_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vunzip_16_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_ee_vunzip_32_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vunzip_32_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_ee_vunzip_8_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_vunzip_8_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_ee_zero_q_qa_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_mov_s16_qacc_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_mov_s8_qacc_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_mov_u16_qacc_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_mov_u8_qacc_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_qa_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_qa_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_cal_double_q_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_semantic_cal_double_q_qu_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 11) >> 31);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie = (tie << 1) | ((insn << 18) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 9) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_qv_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_sel4_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_fld_semantic_cal_double_q_sel4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_fld_semantic_cal_double_q_qy_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 17) >> 31);
    tie = (tie << 2) | ((insn << 19) >> 30);
    tie
}

fn field_fld_semantic_cal_double_q_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_qx_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_qz_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_qz_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_qs1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_qs0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_imm16_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 7) >> 30);
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_cal_double_q_ad_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_cal_double_q_sel8_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_sel8_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_semantic_cal_double_q_sel16_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie = (tie << 1) | ((insn << 27) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_sel16_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie = (tie << 1) | ((insn << 23) >> 31);
    tie
}

fn field_fld_semantic_srcmb_qacc_sel2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 21) >> 31);
    tie
}

fn field_fld_semantic_srcmb_qacc_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_srcmb_qacc_qu_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_srs_accx_sel2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 17) >> 31);
    tie
}

fn field_fld_ee_srs_accx_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_srs_accx_au_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_semantic_cal_double_q_ax_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_semantic_cal_double_q_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_cal_double_q_ay_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_vlogic_qy_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie = (tie << 1) | ((insn << 26) >> 31);
    tie
}

fn field_fld_semantic_vlogic_qx_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie = (tie << 1) | ((insn << 27) >> 31);
    tie
}

fn field_fld_semantic_vlogic_qa_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_shfq_sar16_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_shfq_qs0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_semantic_shfq_qs0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_semantic_shfq_qs1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_shfq_qs1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_semantic_shfq_ad_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 20) >> 28);
    tie
}

fn field_fld_semantic_shfq_ad_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_shfq_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_semantic_shfq_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_semantic_shfq_imm16_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 9) >> 29);
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie = (tie << 2) | ((insn << 26) >> 30);
    tie
}

fn field_fld_semantic_shfq_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_semantic_shfq_qa_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie
}

fn field_fld_semantic_shfq_qs_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_fft_rxbf_sar4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 19) >> 30);
    tie
}

fn field_fld_semantic_fft_rxbf_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_semantic_fft_rxbf_sel2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 23) >> 31);
    tie
}

fn field_fld_semantic_fft_rxbf_qy_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 20) >> 30);
    tie = (tie << 1) | ((insn << 26) >> 31);
    tie
}

fn field_fld_semantic_fft_rxbf_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_semantic_fft_rxbf_qx_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 24) >> 30);
    tie = (tie << 1) | ((insn << 27) >> 31);
    tie
}

fn field_fld_semantic_fft_rxbf_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_semantic_fft_rxbf_qa1_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 17) >> 29);
    tie
}

fn field_fld_semantic_fft_rxbf_qa0_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_semantic_fft_rxbf_qa0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_sel8_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 9) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_qz_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_ad_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_ee_fft_cmul_s16_ld_xp_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_sar4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 8) >> 30);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_upd4_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_sel8_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_ad_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_qv_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_fft_cmul_s16_st_xp_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_ee_bitrev_ax_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_bitrev_qa_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_sel2_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 9) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_qm_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_qz1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_qz_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_sel2_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 9) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_qm_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_qz1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_qz_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_incp_uaup_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_sel2_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 9) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_qm_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_qz1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 24) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_qz_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_ee_fft_ams_s16_ld_r32_decp_qu_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 25) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_sel2_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 8) >> 31);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_qm_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 18) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_qy_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 15) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_qx_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 12) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_as_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_as0_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 28) >> 28);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_qz1_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 9) >> 29);
    tie
}

fn field_fld_ee_fft_ams_s16_st_incp_qv_slot_slot_format_32_0(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 3) | ((insn << 21) >> 29);
    tie
}

fn field_fld_ee_fft_vst_r32_decp_sar2_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 1) | ((insn << 21) >> 31);
    tie
}

fn field_fld_ee_fft_vst_r32_decp_as_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 4) | ((insn << 24) >> 28);
    tie
}

fn field_fld_ee_fft_vst_r32_decp_qv_slot_inst(insn: u32) -> u32 {
    let mut tie: u32 = 0;
    tie = (tie << 2) | ((insn << 10) >> 30);
    tie = (tie << 1) | ((insn << 16) >> 31);
    tie
}

fn implicit_field_ar0(_insn: u32) -> u32 {
    0
}

fn implicit_field_ar4(_insn: u32) -> u32 {
    4
}

fn implicit_field_ar8(_insn: u32) -> u32 {
    8
}

fn implicit_field_ar12(_insn: u32) -> u32 {
    12
}

fn implicit_field_mr0(_insn: u32) -> u32 {
    0
}

fn implicit_field_mr1(_insn: u32) -> u32 {
    1
}

fn implicit_field_mr2(_insn: u32) -> u32 {
    2
}

fn implicit_field_mr3(_insn: u32) -> u32 {
    3
}

fn implicit_field_bt16(_insn: u32) -> u32 {
    0
}

fn implicit_field_bs16(_insn: u32) -> u32 {
    0
}

fn implicit_field_br16(_insn: u32) -> u32 {
    0
}

fn implicit_field_brall(_insn: u32) -> u32 {
    0
}

static FIELD_0: [Option<fn(u32) -> u32>; 263] = [
    Some(field_t_slot_inst),
    Some(field_bbi4_slot_inst),
    Some(field_bbi_slot_inst),
    Some(field_imm12_slot_inst),
    Some(field_imm8_slot_inst),
    Some(field_s_slot_inst),
    Some(field_imm12b_slot_inst),
    Some(field_imm16_slot_inst),
    Some(field_m_slot_inst),
    Some(field_n_slot_inst),
    Some(field_offset_slot_inst),
    Some(field_op0_slot_inst),
    Some(field_op1_slot_inst),
    Some(field_op2_slot_inst),
    Some(field_r_slot_inst),
    Some(field_sa4_slot_inst),
    Some(field_sae4_slot_inst),
    Some(field_sae_slot_inst),
    Some(field_sal_slot_inst),
    Some(field_sargt_slot_inst),
    Some(field_sas4_slot_inst),
    Some(field_sas_slot_inst),
    Some(field_sr_slot_inst),
    Some(field_st_slot_inst),
    Some(field_thi3_slot_inst),
    Some(field_imm4_slot_inst),
    Some(field_mn_slot_inst),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_r3_slot_inst),
    Some(field_rbit2_slot_inst),
    Some(field_rhi_slot_inst),
    Some(field_t3_slot_inst),
    Some(field_tbit2_slot_inst),
    Some(field_tlo_slot_inst),
    Some(field_w_slot_inst),
    Some(field_y_slot_inst),
    Some(field_x_slot_inst),
    Some(field_t2_slot_inst),
    Some(field_s2_slot_inst),
    Some(field_r2_slot_inst),
    Some(field_t4_slot_inst),
    Some(field_s4_slot_inst),
    Some(field_r4_slot_inst),
    Some(field_t8_slot_inst),
    Some(field_s8_slot_inst),
    Some(field_r8_slot_inst),
    Some(field_xt_wbr15_imm_slot_inst),
    Some(field_xt_wbr18_imm_slot_inst),
    None,
    None,
    Some(field_fld_st_qr_b_slot_inst),
    Some(field_fld_st_qr_a_slot_inst),
    Some(field_fld_ld_qr_b_slot_inst),
    Some(field_fld_ld_qr_a_slot_inst),
    Some(field_fld_qr_slot_inst),
    Some(field_fld_inst_4_0_slot_inst),
    Some(field_fld_inst_9_6_slot_inst),
    Some(field_fld_mv_qr_b_slot_inst),
    Some(field_fld_mv_qr_a_slot_inst),
    Some(field_fld_ee_wr_mask_gpio_out_mask_slot_inst),
    Some(field_fld_ee_wr_mask_gpio_out_value_slot_inst),
    Some(field_fld_ee_set_bit_gpio_out_mask_slot_inst),
    Some(field_fld_ee_clr_bit_gpio_out_clear_slot_inst),
    Some(field_fld_inst_23_8_slot_inst),
    Some(field_fld_ee_get_gpio_in_au_slot_inst),
    Some(field_fld_inst_23_22_slot_inst),
    Some(field_fld_semantic_ld_st_imm2_slot_inst),
    Some(field_fld_inst_14_8_slot_inst),
    Some(field_fld_semantic_ld_st_imm1_slot_inst),
    Some(field_fld_inst_11_8_slot_inst),
    Some(field_fld_semantic_ld_st_qu1_slot_inst),
    None,
    Some(field_fld_semantic_ld_st_as_slot_inst),
    Some(field_fld_semantic_ld_st_fu0_slot_inst),
    Some(field_fld_semantic_ld_st_fu1_slot_inst),
    None,
    None,
    None,
    Some(field_fld_semantic_ld_st_ad_slot_inst),
    Some(field_fld_semantic_ld_st_imm8_slot_inst),
    None,
    Some(field_fld_inst_3_0_slot_inst),
    Some(field_fld_inst_19_16_slot_inst),
    Some(field_fld_inst_23_23_slot_inst),
    Some(field_fld_inst_14_12_slot_inst),
    Some(field_fld_inst_21_15_slot_inst),
    Some(field_fld_semantic_ld_st_imm16_slot_inst),
    Some(field_fld_inst_23_12_slot_inst),
    Some(field_fld_semantic_ld_st_imm4_slot_inst),
    None,
    None,
    None,
    Some(field_fld_semantic_ld_st_qu_slot_inst),
    None,
    Some(field_fld_semantic_ld_st_fv0_slot_inst),
    Some(field_fld_semantic_ld_st_fv1_slot_inst),
    None,
    None,
    Some(field_fld_semantic_ld_st_qv_slot_inst),
    Some(field_fld_ee_movi_32_a_sel4_slot_inst),
    Some(field_fld_ee_movi_32_a_au_slot_inst),
    Some(field_fld_ee_movi_32_a_qs_slot_inst),
    Some(field_fld_ee_movi_32_q_sel4_slot_inst),
    Some(field_fld_ee_movi_32_q_as_slot_inst),
    Some(field_fld_ee_movi_32_q_qu_slot_inst),
    Some(field_fld_ee_vzip_16_qs1_slot_inst),
    Some(field_fld_ee_vzip_16_qs0_slot_inst),
    Some(field_fld_ee_vzip_32_qs1_slot_inst),
    Some(field_fld_ee_vzip_32_qs0_slot_inst),
    Some(field_fld_ee_vzip_8_qs1_slot_inst),
    Some(field_fld_ee_vzip_8_qs0_slot_inst),
    Some(field_fld_inst_11_0_slot_inst),
    Some(field_fld_ee_vunzip_16_qs1_slot_inst),
    Some(field_fld_ee_vunzip_16_qs0_slot_inst),
    Some(field_fld_ee_vunzip_32_qs1_slot_inst),
    Some(field_fld_ee_vunzip_32_qs0_slot_inst),
    Some(field_fld_ee_vunzip_8_qs1_slot_inst),
    Some(field_fld_ee_vunzip_8_qs0_slot_inst),
    Some(field_fld_inst_23_0_slot_inst),
    Some(field_fld_ee_zero_q_qa_slot_inst),
    Some(field_fld_inst_14_0_slot_inst),
    Some(field_fld_ee_mov_s16_qacc_qs_slot_inst),
    Some(field_fld_ee_mov_s8_qacc_qs_slot_inst),
    Some(field_fld_ee_mov_u16_qacc_qs_slot_inst),
    Some(field_fld_ee_mov_u8_qacc_qs_slot_inst),
    None,
    Some(field_fld_inst_7_0_slot_inst),
    Some(field_fld_semantic_cal_double_q_qa_slot_inst),
    Some(field_fld_semantic_cal_double_q_as_slot_inst),
    Some(field_fld_semantic_cal_double_q_qu_slot_inst),
    None,
    None,
    None,
    Some(field_fld_inst_7_6_slot_inst),
    Some(field_fld_semantic_cal_double_q_sel4_slot_inst),
    Some(field_fld_semantic_cal_double_q_qy_slot_inst),
    Some(field_fld_semantic_cal_double_q_qx_slot_inst),
    Some(field_fld_semantic_cal_double_q_qz_slot_inst),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_inst_23_21_slot_inst),
    Some(field_fld_semantic_cal_double_q_sel8_slot_inst),
    None,
    Some(field_fld_inst_7_5_slot_inst),
    Some(field_fld_semantic_cal_double_q_sel16_slot_inst),
    Some(field_fld_inst_14_11_slot_inst),
    Some(field_fld_semantic_srcmb_qacc_sel2_slot_inst),
    Some(field_fld_semantic_srcmb_qacc_as_slot_inst),
    Some(field_fld_semantic_srcmb_qacc_qu_slot_inst),
    Some(field_fld_inst_13_12_slot_inst),
    Some(field_fld_inst_23_15_slot_inst),
    Some(field_fld_ee_srs_accx_sel2_slot_inst),
    Some(field_fld_ee_srs_accx_as_slot_inst),
    Some(field_fld_ee_srs_accx_au_slot_inst),
    Some(field_fld_semantic_cal_double_q_ax_slot_inst),
    Some(field_fld_semantic_cal_double_q_qs_slot_inst),
    Some(field_fld_inst_13_13_slot_inst),
    Some(field_fld_semantic_cal_double_q_ay_slot_inst),
    Some(field_fld_inst_9_8_slot_inst),
    Some(field_fld_semantic_vlogic_qy_slot_inst),
    Some(field_fld_semantic_vlogic_qx_slot_inst),
    Some(field_fld_semantic_vlogic_qa_slot_inst),
    Some(field_fld_inst_5_5_slot_inst),
    Some(field_fld_semantic_shfq_sar16_slot_inst),
    Some(field_fld_semantic_shfq_qs0_slot_inst),
    Some(field_fld_semantic_shfq_qs1_slot_inst),
    Some(field_fld_semantic_shfq_ad_slot_inst),
    Some(field_fld_semantic_shfq_as_slot_inst),
    None,
    None,
    None,
    None,
    Some(field_fld_inst_11_7_slot_inst),
    Some(field_fld_semantic_shfq_qa_slot_inst),
    Some(field_fld_inst_14_7_slot_inst),
    Some(field_fld_semantic_shfq_qs_slot_inst),
    None,
    None,
    None,
    Some(field_fld_inst_9_9_slot_inst),
    Some(field_fld_semantic_fft_rxbf_sel2_slot_inst),
    Some(field_fld_semantic_fft_rxbf_qy_slot_inst),
    Some(field_fld_semantic_fft_rxbf_qx_slot_inst),
    Some(field_fld_semantic_fft_rxbf_qa1_slot_inst),
    Some(field_fld_semantic_fft_rxbf_qa0_slot_inst),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_ee_bitrev_ax_slot_inst),
    Some(field_fld_ee_bitrev_qa_slot_inst),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_ee_fft_vst_r32_decp_sar2_slot_inst),
    Some(field_fld_ee_fft_vst_r32_decp_as_slot_inst),
    Some(field_fld_ee_fft_vst_r32_decp_qv_slot_inst),
    Some(implicit_field_ar0),
    Some(implicit_field_ar4),
    Some(implicit_field_ar8),
    Some(implicit_field_ar12),
    Some(implicit_field_mr0),
    Some(implicit_field_mr1),
    Some(implicit_field_mr2),
    Some(implicit_field_mr3),
    Some(implicit_field_bt16),
    Some(implicit_field_bs16),
    Some(implicit_field_br16),
    Some(implicit_field_brall),
];

static FIELD_1: [Option<fn(u32) -> u32>; 263] = [
    Some(field_t_slot_inst16a),
    None,
    None,
    None,
    None,
    Some(field_s_slot_inst16a),
    None,
    None,
    None,
    None,
    None,
    Some(field_op0_slot_inst16a),
    None,
    None,
    Some(field_r_slot_inst16a),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_sr_slot_inst16a),
    Some(field_st_slot_inst16a),
    None,
    Some(field_imm4_slot_inst16a),
    None,
    Some(field_i_slot_inst16a),
    Some(field_imm6lo_slot_inst16a),
    Some(field_imm6hi_slot_inst16a),
    Some(field_imm7lo_slot_inst16a),
    Some(field_imm7hi_slot_inst16a),
    Some(field_z_slot_inst16a),
    Some(field_imm6_slot_inst16a),
    Some(field_imm7_slot_inst16a),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_t2_slot_inst16a),
    Some(field_s2_slot_inst16a),
    Some(field_r2_slot_inst16a),
    Some(field_t4_slot_inst16a),
    Some(field_s4_slot_inst16a),
    Some(field_r4_slot_inst16a),
    Some(field_t8_slot_inst16a),
    Some(field_s8_slot_inst16a),
    Some(field_r8_slot_inst16a),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(implicit_field_ar0),
    Some(implicit_field_ar4),
    Some(implicit_field_ar8),
    Some(implicit_field_ar12),
    Some(implicit_field_mr0),
    Some(implicit_field_mr1),
    Some(implicit_field_mr2),
    Some(implicit_field_mr3),
    Some(implicit_field_bt16),
    Some(implicit_field_bs16),
    Some(implicit_field_br16),
    Some(implicit_field_brall),
];

static FIELD_2: [Option<fn(u32) -> u32>; 263] = [
    Some(field_t_slot_inst16b),
    None,
    None,
    None,
    None,
    Some(field_s_slot_inst16b),
    None,
    None,
    None,
    None,
    None,
    Some(field_op0_slot_inst16b),
    None,
    None,
    Some(field_r_slot_inst16b),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_sr_slot_inst16b),
    Some(field_st_slot_inst16b),
    None,
    Some(field_imm4_slot_inst16b),
    None,
    Some(field_i_slot_inst16b),
    Some(field_imm6lo_slot_inst16b),
    Some(field_imm6hi_slot_inst16b),
    Some(field_imm7lo_slot_inst16b),
    Some(field_imm7hi_slot_inst16b),
    Some(field_z_slot_inst16b),
    Some(field_imm6_slot_inst16b),
    Some(field_imm7_slot_inst16b),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_t2_slot_inst16b),
    Some(field_s2_slot_inst16b),
    Some(field_r2_slot_inst16b),
    Some(field_t4_slot_inst16b),
    Some(field_s4_slot_inst16b),
    Some(field_r4_slot_inst16b),
    Some(field_t8_slot_inst16b),
    Some(field_s8_slot_inst16b),
    Some(field_r8_slot_inst16b),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(implicit_field_ar0),
    Some(implicit_field_ar4),
    Some(implicit_field_ar8),
    Some(implicit_field_ar12),
    Some(implicit_field_mr0),
    Some(implicit_field_mr1),
    Some(implicit_field_mr2),
    Some(implicit_field_mr3),
    Some(implicit_field_bt16),
    Some(implicit_field_bs16),
    Some(implicit_field_br16),
    Some(implicit_field_brall),
];

static FIELD_3: [Option<fn(u32) -> u32>; 263] = [
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_slot_format_32_0_3_0_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_28_4_slot_slot_format_32_0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_semantic_ld_st_imm16f_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_as_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fu0_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fu1_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fu2_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fu3_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_28_24_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_ad_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_imm8_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_7_5_slot_slot_format_32_0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_semantic_ld_st_sel8_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_sel4_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_qs_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_qu_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_28_22_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fv0_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fv1_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fv2_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_fv3_slot_slot_format_32_0),
    Some(field_fld_semantic_ld_st_qv_slot_slot_format_32_0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_slot_format_32_0_10_4_slot_slot_format_32_0),
    None,
    Some(field_fld_semantic_cal_double_q_qa_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_as_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qu_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_10_6_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qv_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_28_20_slot_slot_format_32_0),
    None,
    Some(field_fld_semantic_cal_double_q_sel4_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qy_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qx_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qz_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qs1_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_qs0_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_28_25_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_imm16_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_10_8_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_19_17_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_28_23_slot_slot_format_32_0),
    Some(field_fld_semantic_cal_double_q_ad_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_7_4_slot_slot_format_32_0),
    None,
    Some(field_fld_semantic_cal_double_q_sel8_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_10_9_slot_slot_format_32_0),
    None,
    Some(field_fld_semantic_cal_double_q_sel16_slot_slot_format_32_0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(field_fld_semantic_shfq_qs0_slot_slot_format_32_0),
    Some(field_fld_semantic_shfq_qs1_slot_slot_format_32_0),
    Some(field_fld_semantic_shfq_ad_slot_slot_format_32_0),
    Some(field_fld_semantic_shfq_as_slot_slot_format_32_0),
    Some(field_fld_semantic_shfq_imm16_slot_slot_format_32_0),
    Some(field_fld_semantic_shfq_qu_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_7_6_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_13_11_slot_slot_format_32_0),
    None,
    None,
    None,
    None,
    Some(field_fld_semantic_fft_rxbf_sar4_slot_slot_format_32_0),
    Some(field_fld_semantic_fft_rxbf_as_slot_slot_format_32_0),
    Some(field_fld_slot_format_32_0_13_13_slot_slot_format_32_0),
    None,
    None,
    Some(field_fld_semantic_fft_rxbf_qy_slot_slot_format_32_0),
    Some(field_fld_semantic_fft_rxbf_qx_slot_slot_format_32_0),
    None,
    Some(field_fld_semantic_fft_rxbf_qa0_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_sel8_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_qy_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_qx_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_qz_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_ad_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_as_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_ld_xp_qu_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_sar4_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_upd4_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_sel8_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_ad_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_as_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_qv_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_qy_slot_slot_format_32_0),
    Some(field_fld_ee_fft_cmul_s16_st_xp_qx_slot_slot_format_32_0),
    None,
    None,
    Some(field_fld_ee_fft_ams_s16_ld_incp_sel2_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_qm_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_qy_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_qx_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_qz1_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_qz_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_as_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_qu_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_sel2_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_qm_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_qy_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_qx_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_qz1_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_qz_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_as_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_incp_uaup_qu_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_sel2_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_qm_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_qy_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_qx_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_qz1_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_qz_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_as_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_ld_r32_decp_qu_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_sel2_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_qm_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_qy_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_qx_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_as_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_as0_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_qz1_slot_slot_format_32_0),
    Some(field_fld_ee_fft_ams_s16_st_incp_qv_slot_slot_format_32_0),
    None,
    None,
    None,
    Some(implicit_field_ar0),
    Some(implicit_field_ar4),
    Some(implicit_field_ar8),
    Some(implicit_field_ar12),
    Some(implicit_field_mr0),
    Some(implicit_field_mr1),
    Some(implicit_field_mr2),
    Some(implicit_field_mr3),
    Some(implicit_field_bt16),
    Some(implicit_field_bs16),
    Some(implicit_field_br16),
    Some(implicit_field_brall),
];

pub(crate) static FIELD: [&[Option<fn(u32) -> u32>]; 4] = [
    &FIELD_0,
    &FIELD_1,
    &FIELD_2,
    &FIELD_3,
];

pub(crate) fn format_len(b0: u8) -> Option<(u8, u8)> {
    let insn = b0 as u32;
    if (insn & 0x8) == 0 {
        return Some((3, 0));
    }
    if (insn & 0xc) == 0x8 {
        return Some((2, 1));
    }
    if (insn & 0xe) == 0xc {
        return Some((2, 2));
    }
    if (insn & 0xe) == 0xe {
        return Some((4, 3));
    }
    None
}

pub(crate) fn extract_slot(slot: u8, insn: u32) -> u32 {
    match slot {
        0 => {
            let slot: u32 = ((insn) & (0xffffffu32));
            slot
        }
        1 => {
            let slot: u32 = ((insn) & (0xffffu32));
            slot
        }
        2 => {
            let slot: u32 = ((insn) & (0xffffu32));
            slot
        }
        3 => {
            let mut slot: u32 = ((((insn) & (0xff0u32))) >> (0x4u32));
            slot = ((((slot) & ((!(0xf00u32))))) | (((((((insn) & (0xf00000u32))) >> (0x14u32))) << (0x8u32))));
            slot = ((((slot) & ((!(0xf000u32))))) | (((((((insn) & (0xf000u32))) >> (0xcu32))) << (0xcu32))));
            slot = ((((slot) & ((!(0x10000u32))))) | (((((insn) & (0x1u32))) << (0x10u32))));
            slot = ((((slot) & ((!(0x1e0000u32))))) | (((((((insn) & (0xf0000u32))) >> (0x10u32))) << (0x11u32))));
            slot = ((((slot) & ((!(0x1fe00000u32))))) | (((((((insn) & (0xff000000u32))) >> (0x18u32))) << (0x15u32))));
            slot
        }
        _ => insn,
    }
}

pub(crate) fn decode_slot(slot: u8, insn: u32) -> SlotResult {
    match slot {
        0 => slot_inst(insn),
        1 => slot_inst16a(insn),
        2 => slot_inst16b(insn),
        3 => slot_slot_format_32_0(insn),
        _ => SlotResult::Undefined,
    }
}

pub(crate) fn slot_inst(insn: u32) -> SlotResult {
    if (field_fld_inst_19_16_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 0) {
        return SlotResult::Op(Op::EE_LDF_64_XP);
    }
    if (field_fld_inst_19_16_slot_inst(insn) == 7 && field_fld_inst_3_0_slot_inst(insn) == 0) {
        return SlotResult::Op(Op::EE_STF_64_XP);
    }
    if (field_fld_inst_23_0_slot_inst(insn) == 2426884) {
        return SlotResult::Op(Op::EE_ZERO_ACCX);
    }
    if (field_fld_inst_23_0_slot_inst(insn) == 2426948) {
        return SlotResult::Op(Op::EE_ZERO_QACC);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 1796 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_U8_128_XP);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 1812 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_S8_128_XP);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 1828 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_WR_MASK_GPIO_OUT);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 1876 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SET_BIT_GPIO_OUT);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 1892 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_CLR_BIT_GPIO_OUT);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 1956 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_U16_128_XP);
    }
    if (field_fld_inst_23_12_slot_inst(insn) == 2020 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_S16_128_XP);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 20 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 132) {
        return SlotResult::Op(Op::EE_VMULAS_U16_ACCX);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 20 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 196) {
        return SlotResult::Op(Op::EE_VMULAS_U8_ACCX);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 20 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 132) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 20 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 196) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 52 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 132) {
        return SlotResult::Op(Op::EE_VMULAS_S16_ACCX);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 52 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 196) {
        return SlotResult::Op(Op::EE_VMULAS_S8_ACCX);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 52 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 132) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 52 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 196) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC);
    }
    if (field_fld_inst_23_15_slot_inst(insn) == 252 && field_fld_inst_13_12_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRS_ACCX);
    }
    if (field_fld_inst_23_21_slot_inst(insn) == 4 && field_fld_inst_19_16_slot_inst(insn) == 7 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC_LDBC_INCP);
    }
    if (field_fld_inst_23_21_slot_inst(insn) == 5 && field_fld_inst_19_16_slot_inst(insn) == 7 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC_LDBC_INCP);
    }
    if (field_fld_inst_23_21_slot_inst(insn) == 6 && field_fld_inst_19_16_slot_inst(insn) == 7 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC_LDBC_INCP);
    }
    if (field_fld_inst_23_21_slot_inst(insn) == 7 && field_fld_inst_19_16_slot_inst(insn) == 7 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC_LDBC_INCP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 15 && field_fld_inst_14_12_slot_inst(insn) == 0 && field_fld_inst_9_6_slot_inst(insn) == 0 && field_fld_inst_4_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::mv_QR);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLD_128_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLD_H_64_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 3 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLD_L_64_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 5 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_16_IP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 4 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_16_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_32_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 5 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_8_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_128_USAR_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 7 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VST_128_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 100) {
        return SlotResult::Op(Op::EE_VADDS_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 116) {
        return SlotResult::Op(Op::EE_VADDS_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 132) {
        return SlotResult::Op(Op::EE_VADDS_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 212) {
        return SlotResult::Op(Op::EE_VSUBS_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 228) {
        return SlotResult::Op(Op::EE_VSUBS_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 244) {
        return SlotResult::Op(Op::EE_VSUBS_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 132) {
        return SlotResult::Op(Op::EE_VMUL_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 148) {
        return SlotResult::Op(Op::EE_VMUL_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 164) {
        return SlotResult::Op(Op::EE_VMUL_U16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 180) {
        return SlotResult::Op(Op::EE_VMUL_U8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_6_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_CMUL_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 196) {
        return SlotResult::Op(Op::EE_VSMULAS_S16_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_5_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VSMULAS_S8_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VPRELU_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VPRELU_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 36) {
        return SlotResult::Op(Op::EE_VMAX_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 52) {
        return SlotResult::Op(Op::EE_VMAX_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 68) {
        return SlotResult::Op(Op::EE_VMAX_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 84) {
        return SlotResult::Op(Op::EE_VMIN_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 100) {
        return SlotResult::Op(Op::EE_VMIN_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 116) {
        return SlotResult::Op(Op::EE_VMIN_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 148) {
        return SlotResult::Op(Op::EE_VCMP_EQ_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 164) {
        return SlotResult::Op(Op::EE_VCMP_EQ_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 180) {
        return SlotResult::Op(Op::EE_VCMP_EQ_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 196) {
        return SlotResult::Op(Op::EE_VCMP_GT_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 212) {
        return SlotResult::Op(Op::EE_VCMP_GT_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 228) {
        return SlotResult::Op(Op::EE_VCMP_GT_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 0 && field_fld_inst_7_0_slot_inst(insn) == 244) {
        return SlotResult::Op(Op::EE_VCMP_LT_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VCMP_LT_S32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 14 && field_fld_inst_13_13_slot_inst(insn) == 1 && field_fld_inst_7_0_slot_inst(insn) == 20) {
        return SlotResult::Op(Op::EE_VCMP_LT_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 2 && field_fld_inst_19_16_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SLCXXP_2Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::st_QR);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::ld_QR);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_8_slot_inst(insn) == 115 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_8_slot_inst(insn) == 119 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 5 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_8_IP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_8_slot_inst(insn) == 59 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_8_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDHBC_16_INCP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VST_H_64_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 4 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VST_L_64_XP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 7 && field_fld_inst_9_8_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_MOVI_32_A);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 3 && field_fld_inst_9_8_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_MOVI_32_Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_0_slot_inst(insn) == 948) {
        return SlotResult::Op(Op::EE_VZIP_16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_0_slot_inst(insn) == 964) {
        return SlotResult::Op(Op::EE_VZIP_32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_0_slot_inst(insn) == 980) {
        return SlotResult::Op(Op::EE_VZIP_8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_0_slot_inst(insn) == 900) {
        return SlotResult::Op(Op::EE_VUNZIP_16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_0_slot_inst(insn) == 916) {
        return SlotResult::Op(Op::EE_VUNZIP_32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_0_slot_inst(insn) == 932) {
        return SlotResult::Op(Op::EE_VUNZIP_8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_0_slot_inst(insn) == 32676) {
        return SlotResult::Op(Op::EE_ZERO_Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_0_slot_inst(insn) == 32548) {
        return SlotResult::Op(Op::EE_MOV_S16_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_0_slot_inst(insn) == 32564) {
        return SlotResult::Op(Op::EE_MOV_S8_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_0_slot_inst(insn) == 32612) {
        return SlotResult::Op(Op::EE_MOV_U16_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_0_slot_inst(insn) == 32628) {
        return SlotResult::Op(Op::EE_MOV_U8_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_11_slot_inst(insn) == 14 && field_fld_inst_9_8_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRCMB_S16_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_11_slot_inst(insn) == 15 && field_fld_inst_9_8_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRCMB_S8_QACC);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VRELU_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 5 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VRELU_S8);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 3 && field_fld_inst_9_8_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ANDQ);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_8_slot_inst(insn) == 127 && field_fld_inst_5_5_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_NOTQ);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 7 && field_fld_inst_9_8_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ORQ);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_12_slot_inst(insn) == 3 && field_fld_inst_9_8_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_XORQ);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_8_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SLCI_2Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_8_slot_inst(insn) == 10 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRCI_2Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_8_slot_inst(insn) == 14 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRCQ_128_ST_INCP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_7_slot_inst(insn) == 14 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRC_Q_QUP);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_11_7_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRC_Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 6 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_SRCXXP_2Q);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_7_slot_inst(insn) == 126 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VSL_32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_7_slot_inst(insn) == 127 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VSR_32);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 12 && field_fld_inst_9_9_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_FFT_R2BF_S16);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_8_slot_inst(insn) == 123 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_BITREV);
    }
    if (field_fld_inst_23_22_slot_inst(insn) == 3 && field_fld_inst_19_16_slot_inst(insn) == 13 && field_fld_inst_14_11_slot_inst(insn) == 6 && field_fld_inst_9_8_slot_inst(insn) == 3 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_FFT_VST_R32_DECP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_S16_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 34 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_S8_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 10 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_U16_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 42 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LDQA_U8_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 60 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_QACC_H_H_32_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 12 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_QACC_H_L_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 44 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_QACC_L_H_32_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 0 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_QACC_L_L_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 28 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_ACCX_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 32 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_UA_STATE_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 56 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ST_UA_STATE_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 36 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ST_QACC_H_H_32_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 26 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ST_QACC_H_L_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 58 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ST_QACC_L_H_32_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 24 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ST_QACC_L_L_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 0 && field_fld_inst_21_15_slot_inst(insn) == 4 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_ST_ACCX_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 3 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLD_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 8 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLD_H_64_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 9 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLD_L_64_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 2 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VLDBC_32_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 1 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_LD_128_USAR_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 10 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VST_128_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 11 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VST_H_64_IP);
    }
    if (field_fld_inst_23_23_slot_inst(insn) == 1 && field_fld_inst_19_16_slot_inst(insn) == 4 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_VST_L_64_IP);
    }
    if (field_fld_inst_23_8_slot_inst(insn) == 25864 && field_fld_inst_3_0_slot_inst(insn) == 4) {
        return SlotResult::Op(Op::EE_GET_GPIO_IN);
    }
    if (field_op0_slot_inst(insn) == 0) {
        if (field_op1_slot_inst(insn) == 0) {
            if (field_op2_slot_inst(insn) == 0) {
                if (field_r_slot_inst(insn) == 0) {
                    if (field_m_slot_inst(insn) == 0 && field_s_slot_inst(insn) == 0 && field_n_slot_inst(insn) == 0) {
                        return SlotResult::Op(Op::ILL);
                    }
                    if (field_m_slot_inst(insn) == 2) {
                        if (field_n_slot_inst(insn) == 0) {
                            return SlotResult::Op(Op::RET);
                        }
                        if (field_n_slot_inst(insn) == 1) {
                            return SlotResult::Op(Op::RETW);
                        }
                        if (field_n_slot_inst(insn) == 2) {
                            return SlotResult::Op(Op::JX);
                        }
                    }
                    if (field_m_slot_inst(insn) == 3) {
                        if (field_n_slot_inst(insn) == 0) {
                            return SlotResult::Op(Op::CALLX0);
                        }
                        if (field_n_slot_inst(insn) == 1) {
                            return SlotResult::Op(Op::CALLX4);
                        }
                        if (field_n_slot_inst(insn) == 2) {
                            return SlotResult::Op(Op::CALLX8);
                        }
                        if (field_n_slot_inst(insn) == 3) {
                            return SlotResult::Op(Op::CALLX12);
                        }
                    }
                }
                if (field_r_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::MOVSP);
                }
                if (field_r_slot_inst(insn) == 2) {
                    if (field_s_slot_inst(insn) == 0) {
                        if (field_t_slot_inst(insn) == 0) {
                            return SlotResult::Op(Op::ISYNC);
                        }
                        if (field_t_slot_inst(insn) == 1) {
                            return SlotResult::Op(Op::RSYNC);
                        }
                        if (field_t_slot_inst(insn) == 2) {
                            return SlotResult::Op(Op::ESYNC);
                        }
                        if (field_t_slot_inst(insn) == 3) {
                            return SlotResult::Op(Op::DSYNC);
                        }
                        if (field_t_slot_inst(insn) == 8) {
                            return SlotResult::Op(Op::EXCW);
                        }
                        if (field_t_slot_inst(insn) == 12) {
                            return SlotResult::Op(Op::MEMW);
                        }
                        if (field_t_slot_inst(insn) == 13) {
                            return SlotResult::Op(Op::EXTW);
                        }
                        if (field_t_slot_inst(insn) == 15) {
                            return SlotResult::Op(Op::NOP);
                        }
                    }
                }
                if (field_r_slot_inst(insn) == 3) {
                    if (field_t_slot_inst(insn) == 0) {
                        if (field_s_slot_inst(insn) == 0) {
                            return SlotResult::Op(Op::RFE);
                        }
                        if (field_s_slot_inst(insn) == 2) {
                            return SlotResult::Op(Op::RFDE);
                        }
                        if (field_s_slot_inst(insn) == 4) {
                            return SlotResult::Op(Op::RFWO);
                        }
                        if (field_s_slot_inst(insn) == 5) {
                            return SlotResult::Op(Op::RFWU);
                        }
                    }
                    if (field_t_slot_inst(insn) == 1) {
                        return SlotResult::Op(Op::RFI);
                    }
                }
                if (field_r_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::BREAK);
                }
                if (field_r_slot_inst(insn) == 5) {
                    if (field_s_slot_inst(insn) == 0 && field_t_slot_inst(insn) == 0) {
                        return SlotResult::Op(Op::SYSCALL);
                    }
                    if (field_s_slot_inst(insn) == 1 && field_t_slot_inst(insn) == 0) {
                        return SlotResult::Op(Op::SIMCALL);
                    }
                }
                if (field_r_slot_inst(insn) == 6) {
                    return SlotResult::Op(Op::RSIL);
                }
                if (field_r_slot_inst(insn) == 7 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::WAITI);
                }
                if (field_r_slot_inst(insn) == 7) {
                    if (field_t_slot_inst(insn) == 14) {
                        return SlotResult::Op(Op::LDDR32_P);
                    }
                    if (field_t_slot_inst(insn) == 15) {
                        return SlotResult::Op(Op::SDDR32_P);
                    }
                }
                if (field_r_slot_inst(insn) == 8) {
                    return SlotResult::Op(Op::ANY4);
                }
                if (field_r_slot_inst(insn) == 9) {
                    return SlotResult::Op(Op::ALL4);
                }
                if (field_r_slot_inst(insn) == 10) {
                    return SlotResult::Op(Op::ANY8);
                }
                if (field_r_slot_inst(insn) == 11) {
                    return SlotResult::Op(Op::ALL8);
                }
            }
            if (field_op2_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::AND);
            }
            if (field_op2_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::OR);
            }
            if (field_op2_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::XOR);
            }
            if (field_op2_slot_inst(insn) == 4) {
                if (field_r_slot_inst(insn) == 0 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::SSR);
                }
                if (field_r_slot_inst(insn) == 1 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::SSL);
                }
                if (field_r_slot_inst(insn) == 2 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::SSA8L);
                }
                if (field_r_slot_inst(insn) == 3 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::SSA8B);
                }
                if (field_r_slot_inst(insn) == 4 && field_thi3_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::SSAI);
                }
                if (field_r_slot_inst(insn) == 6) {
                    return SlotResult::Op(Op::RER);
                }
                if (field_r_slot_inst(insn) == 7) {
                    return SlotResult::Op(Op::WER);
                }
                if (field_r_slot_inst(insn) == 8 && field_s_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::ROTW);
                }
                if (field_r_slot_inst(insn) == 14) {
                    return SlotResult::Op(Op::NSA);
                }
                if (field_r_slot_inst(insn) == 15) {
                    return SlotResult::Op(Op::NSAU);
                }
            }
            if (field_op2_slot_inst(insn) == 5) {
                if (field_r_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::RITLB0);
                }
                if (field_r_slot_inst(insn) == 4 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::IITLB);
                }
                if (field_r_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::PITLB);
                }
                if (field_r_slot_inst(insn) == 6) {
                    return SlotResult::Op(Op::WITLB);
                }
                if (field_r_slot_inst(insn) == 7) {
                    return SlotResult::Op(Op::RITLB1);
                }
                if (field_r_slot_inst(insn) == 11) {
                    return SlotResult::Op(Op::RDTLB0);
                }
                if (field_r_slot_inst(insn) == 12 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::IDTLB);
                }
                if (field_r_slot_inst(insn) == 13) {
                    return SlotResult::Op(Op::PDTLB);
                }
                if (field_r_slot_inst(insn) == 14) {
                    return SlotResult::Op(Op::WDTLB);
                }
                if (field_r_slot_inst(insn) == 15) {
                    return SlotResult::Op(Op::RDTLB1);
                }
            }
            if (field_op2_slot_inst(insn) == 6) {
                if (field_s_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::NEG);
                }
                if (field_s_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::ABS);
                }
            }
            if (field_op2_slot_inst(insn) == 8) {
                return SlotResult::Op(Op::ADD);
            }
            if (field_op2_slot_inst(insn) == 9) {
                return SlotResult::Op(Op::ADDX2);
            }
            if (field_op2_slot_inst(insn) == 10) {
                return SlotResult::Op(Op::ADDX4);
            }
            if (field_op2_slot_inst(insn) == 11) {
                return SlotResult::Op(Op::ADDX8);
            }
            if (field_op2_slot_inst(insn) == 12) {
                return SlotResult::Op(Op::SUB);
            }
            if (field_op2_slot_inst(insn) == 13) {
                return SlotResult::Op(Op::SUBX2);
            }
            if (field_op2_slot_inst(insn) == 14) {
                return SlotResult::Op(Op::SUBX4);
            }
            if (field_op2_slot_inst(insn) == 15) {
                return SlotResult::Op(Op::SUBX8);
            }
        }
        if (field_op1_slot_inst(insn) == 1) {
            if ((field_op2_slot_inst(insn) == 0 || field_op2_slot_inst(insn) == 1)) {
                return SlotResult::Op(Op::SLLI);
            }
            if ((field_op2_slot_inst(insn) == 2 || field_op2_slot_inst(insn) == 3)) {
                return SlotResult::Op(Op::SRAI);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::SRLI);
            }
            if (field_op2_slot_inst(insn) == 6) {
                if (field_sr_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::XSR_LBEG);
                }
                if (field_sr_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::XSR_LEND);
                }
                if (field_sr_slot_inst(insn) == 2) {
                    return SlotResult::Op(Op::XSR_LCOUNT);
                }
                if (field_sr_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::XSR_SAR);
                }
                if (field_sr_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::XSR_BR);
                }
                if (field_sr_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::XSR_LITBASE);
                }
                if (field_sr_slot_inst(insn) == 12) {
                    return SlotResult::Op(Op::XSR_SCOMPARE1);
                }
                if (field_sr_slot_inst(insn) == 16) {
                    return SlotResult::Op(Op::XSR_ACCLO);
                }
                if (field_sr_slot_inst(insn) == 17) {
                    return SlotResult::Op(Op::XSR_ACCHI);
                }
                if (field_sr_slot_inst(insn) == 32) {
                    return SlotResult::Op(Op::XSR_M0);
                }
                if (field_sr_slot_inst(insn) == 33) {
                    return SlotResult::Op(Op::XSR_M1);
                }
                if (field_sr_slot_inst(insn) == 34) {
                    return SlotResult::Op(Op::XSR_M2);
                }
                if (field_sr_slot_inst(insn) == 35) {
                    return SlotResult::Op(Op::XSR_M3);
                }
                if (field_sr_slot_inst(insn) == 72) {
                    return SlotResult::Op(Op::XSR_WINDOWBASE);
                }
                if (field_sr_slot_inst(insn) == 73) {
                    return SlotResult::Op(Op::XSR_WINDOWSTART);
                }
                if (field_sr_slot_inst(insn) == 95) {
                    return SlotResult::Op(Op::XSR_ERACCESS);
                }
                if (field_sr_slot_inst(insn) == 96) {
                    return SlotResult::Op(Op::XSR_IBREAKENABLE);
                }
                if (field_sr_slot_inst(insn) == 97) {
                    return SlotResult::Op(Op::XSR_MEMCTL);
                }
                if (field_sr_slot_inst(insn) == 99) {
                    return SlotResult::Op(Op::XSR_ATOMCTL);
                }
                if (field_sr_slot_inst(insn) == 104) {
                    return SlotResult::Op(Op::XSR_DDR);
                }
                if (field_sr_slot_inst(insn) == 128) {
                    return SlotResult::Op(Op::XSR_IBREAKA0);
                }
                if (field_sr_slot_inst(insn) == 129) {
                    return SlotResult::Op(Op::XSR_IBREAKA1);
                }
                if (field_sr_slot_inst(insn) == 144) {
                    return SlotResult::Op(Op::XSR_DBREAKA0);
                }
                if (field_sr_slot_inst(insn) == 145) {
                    return SlotResult::Op(Op::XSR_DBREAKA1);
                }
                if (field_sr_slot_inst(insn) == 160) {
                    return SlotResult::Op(Op::XSR_DBREAKC0);
                }
                if (field_sr_slot_inst(insn) == 161) {
                    return SlotResult::Op(Op::XSR_DBREAKC1);
                }
                if (field_sr_slot_inst(insn) == 177) {
                    return SlotResult::Op(Op::XSR_EPC1);
                }
                if (field_sr_slot_inst(insn) == 178) {
                    return SlotResult::Op(Op::XSR_EPC2);
                }
                if (field_sr_slot_inst(insn) == 179) {
                    return SlotResult::Op(Op::XSR_EPC3);
                }
                if (field_sr_slot_inst(insn) == 180) {
                    return SlotResult::Op(Op::XSR_EPC4);
                }
                if (field_sr_slot_inst(insn) == 181) {
                    return SlotResult::Op(Op::XSR_EPC5);
                }
                if (field_sr_slot_inst(insn) == 182) {
                    return SlotResult::Op(Op::XSR_EPC6);
                }
                if (field_sr_slot_inst(insn) == 183) {
                    return SlotResult::Op(Op::XSR_EPC7);
                }
                if (field_sr_slot_inst(insn) == 192) {
                    return SlotResult::Op(Op::XSR_DEPC);
                }
                if (field_sr_slot_inst(insn) == 194) {
                    return SlotResult::Op(Op::XSR_EPS2);
                }
                if (field_sr_slot_inst(insn) == 195) {
                    return SlotResult::Op(Op::XSR_EPS3);
                }
                if (field_sr_slot_inst(insn) == 196) {
                    return SlotResult::Op(Op::XSR_EPS4);
                }
                if (field_sr_slot_inst(insn) == 197) {
                    return SlotResult::Op(Op::XSR_EPS5);
                }
                if (field_sr_slot_inst(insn) == 198) {
                    return SlotResult::Op(Op::XSR_EPS6);
                }
                if (field_sr_slot_inst(insn) == 199) {
                    return SlotResult::Op(Op::XSR_EPS7);
                }
                if (field_sr_slot_inst(insn) == 209) {
                    return SlotResult::Op(Op::XSR_EXCSAVE1);
                }
                if (field_sr_slot_inst(insn) == 210) {
                    return SlotResult::Op(Op::XSR_EXCSAVE2);
                }
                if (field_sr_slot_inst(insn) == 211) {
                    return SlotResult::Op(Op::XSR_EXCSAVE3);
                }
                if (field_sr_slot_inst(insn) == 212) {
                    return SlotResult::Op(Op::XSR_EXCSAVE4);
                }
                if (field_sr_slot_inst(insn) == 213) {
                    return SlotResult::Op(Op::XSR_EXCSAVE5);
                }
                if (field_sr_slot_inst(insn) == 214) {
                    return SlotResult::Op(Op::XSR_EXCSAVE6);
                }
                if (field_sr_slot_inst(insn) == 215) {
                    return SlotResult::Op(Op::XSR_EXCSAVE7);
                }
                if (field_sr_slot_inst(insn) == 224) {
                    return SlotResult::Op(Op::XSR_CPENABLE);
                }
                if (field_sr_slot_inst(insn) == 228) {
                    return SlotResult::Op(Op::XSR_INTENABLE);
                }
                if (field_sr_slot_inst(insn) == 230) {
                    return SlotResult::Op(Op::XSR_PS);
                }
                if (field_sr_slot_inst(insn) == 231) {
                    return SlotResult::Op(Op::XSR_VECBASE);
                }
                if (field_sr_slot_inst(insn) == 232) {
                    return SlotResult::Op(Op::XSR_EXCCAUSE);
                }
                if (field_sr_slot_inst(insn) == 233) {
                    return SlotResult::Op(Op::XSR_DEBUGCAUSE);
                }
                if (field_sr_slot_inst(insn) == 234) {
                    return SlotResult::Op(Op::XSR_CCOUNT);
                }
                if (field_sr_slot_inst(insn) == 236) {
                    return SlotResult::Op(Op::XSR_ICOUNT);
                }
                if (field_sr_slot_inst(insn) == 237) {
                    return SlotResult::Op(Op::XSR_ICOUNTLEVEL);
                }
                if (field_sr_slot_inst(insn) == 238) {
                    return SlotResult::Op(Op::XSR_EXCVADDR);
                }
                if (field_sr_slot_inst(insn) == 240) {
                    return SlotResult::Op(Op::XSR_CCOMPARE0);
                }
                if (field_sr_slot_inst(insn) == 241) {
                    return SlotResult::Op(Op::XSR_CCOMPARE1);
                }
                if (field_sr_slot_inst(insn) == 242) {
                    return SlotResult::Op(Op::XSR_CCOMPARE2);
                }
                if (field_sr_slot_inst(insn) == 244) {
                    return SlotResult::Op(Op::XSR_MISC0);
                }
                if (field_sr_slot_inst(insn) == 245) {
                    return SlotResult::Op(Op::XSR_MISC1);
                }
                if (field_sr_slot_inst(insn) == 246) {
                    return SlotResult::Op(Op::XSR_MISC2);
                }
                if (field_sr_slot_inst(insn) == 247) {
                    return SlotResult::Op(Op::XSR_MISC3);
                }
            }
            if (field_op2_slot_inst(insn) == 8) {
                return SlotResult::Op(Op::SRC);
            }
            if (field_op2_slot_inst(insn) == 9 && field_s_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::SRL);
            }
            if (field_op2_slot_inst(insn) == 10 && field_t_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::SLL);
            }
            if (field_op2_slot_inst(insn) == 11 && field_s_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::SRA);
            }
            if (field_op2_slot_inst(insn) == 12) {
                return SlotResult::Op(Op::MUL16U);
            }
            if (field_op2_slot_inst(insn) == 13) {
                return SlotResult::Op(Op::MUL16S);
            }
            if (field_op2_slot_inst(insn) == 15) {
                if (field_r_slot_inst(insn) == 14 && field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::RFDO);
                }
                if (field_r_slot_inst(insn) == 14 && field_t_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::RFDD);
                }
            }
        }
        if (field_op1_slot_inst(insn) == 2) {
            if (field_op2_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::ANDB);
            }
            if (field_op2_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::ANDBC);
            }
            if (field_op2_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::ORB);
            }
            if (field_op2_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::ORBC);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::XORB);
            }
            if (field_op2_slot_inst(insn) == 6) {
                return SlotResult::Op(Op::SALTU);
            }
            if (field_op2_slot_inst(insn) == 7) {
                return SlotResult::Op(Op::SALT);
            }
            if (field_op2_slot_inst(insn) == 8) {
                return SlotResult::Op(Op::MULL);
            }
            if (field_op2_slot_inst(insn) == 10) {
                return SlotResult::Op(Op::MULUH);
            }
            if (field_op2_slot_inst(insn) == 11) {
                return SlotResult::Op(Op::MULSH);
            }
            if (field_op2_slot_inst(insn) == 12) {
                return SlotResult::Op(Op::QUOU);
            }
            if (field_op2_slot_inst(insn) == 13) {
                return SlotResult::Op(Op::QUOS);
            }
            if (field_op2_slot_inst(insn) == 14) {
                return SlotResult::Op(Op::REMU);
            }
            if (field_op2_slot_inst(insn) == 15) {
                return SlotResult::Op(Op::REMS);
            }
        }
        if (field_op1_slot_inst(insn) == 3) {
            if (field_op2_slot_inst(insn) == 0) {
                if (field_sr_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::RSR_LBEG);
                }
                if (field_sr_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::RSR_LEND);
                }
                if (field_sr_slot_inst(insn) == 2) {
                    return SlotResult::Op(Op::RSR_LCOUNT);
                }
                if (field_sr_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::RSR_SAR);
                }
                if (field_sr_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::RSR_BR);
                }
                if (field_sr_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::RSR_LITBASE);
                }
                if (field_sr_slot_inst(insn) == 12) {
                    return SlotResult::Op(Op::RSR_SCOMPARE1);
                }
                if (field_sr_slot_inst(insn) == 16) {
                    return SlotResult::Op(Op::RSR_ACCLO);
                }
                if (field_sr_slot_inst(insn) == 17) {
                    return SlotResult::Op(Op::RSR_ACCHI);
                }
                if (field_sr_slot_inst(insn) == 32) {
                    return SlotResult::Op(Op::RSR_M0);
                }
                if (field_sr_slot_inst(insn) == 33) {
                    return SlotResult::Op(Op::RSR_M1);
                }
                if (field_sr_slot_inst(insn) == 34) {
                    return SlotResult::Op(Op::RSR_M2);
                }
                if (field_sr_slot_inst(insn) == 35) {
                    return SlotResult::Op(Op::RSR_M3);
                }
                if (field_sr_slot_inst(insn) == 72) {
                    return SlotResult::Op(Op::RSR_WINDOWBASE);
                }
                if (field_sr_slot_inst(insn) == 73) {
                    return SlotResult::Op(Op::RSR_WINDOWSTART);
                }
                if (field_sr_slot_inst(insn) == 95) {
                    return SlotResult::Op(Op::RSR_ERACCESS);
                }
                if (field_sr_slot_inst(insn) == 96) {
                    return SlotResult::Op(Op::RSR_IBREAKENABLE);
                }
                if (field_sr_slot_inst(insn) == 97) {
                    return SlotResult::Op(Op::RSR_MEMCTL);
                }
                if (field_sr_slot_inst(insn) == 99) {
                    return SlotResult::Op(Op::RSR_ATOMCTL);
                }
                if (field_sr_slot_inst(insn) == 104) {
                    return SlotResult::Op(Op::RSR_DDR);
                }
                if (field_sr_slot_inst(insn) == 128) {
                    return SlotResult::Op(Op::RSR_IBREAKA0);
                }
                if (field_sr_slot_inst(insn) == 129) {
                    return SlotResult::Op(Op::RSR_IBREAKA1);
                }
                if (field_sr_slot_inst(insn) == 144) {
                    return SlotResult::Op(Op::RSR_DBREAKA0);
                }
                if (field_sr_slot_inst(insn) == 145) {
                    return SlotResult::Op(Op::RSR_DBREAKA1);
                }
                if (field_sr_slot_inst(insn) == 160) {
                    return SlotResult::Op(Op::RSR_DBREAKC0);
                }
                if (field_sr_slot_inst(insn) == 161) {
                    return SlotResult::Op(Op::RSR_DBREAKC1);
                }
                if (field_sr_slot_inst(insn) == 176) {
                    return SlotResult::Op(Op::RSR_CONFIGID0);
                }
                if (field_sr_slot_inst(insn) == 177) {
                    return SlotResult::Op(Op::RSR_EPC1);
                }
                if (field_sr_slot_inst(insn) == 178) {
                    return SlotResult::Op(Op::RSR_EPC2);
                }
                if (field_sr_slot_inst(insn) == 179) {
                    return SlotResult::Op(Op::RSR_EPC3);
                }
                if (field_sr_slot_inst(insn) == 180) {
                    return SlotResult::Op(Op::RSR_EPC4);
                }
                if (field_sr_slot_inst(insn) == 181) {
                    return SlotResult::Op(Op::RSR_EPC5);
                }
                if (field_sr_slot_inst(insn) == 182) {
                    return SlotResult::Op(Op::RSR_EPC6);
                }
                if (field_sr_slot_inst(insn) == 183) {
                    return SlotResult::Op(Op::RSR_EPC7);
                }
                if (field_sr_slot_inst(insn) == 192) {
                    return SlotResult::Op(Op::RSR_DEPC);
                }
                if (field_sr_slot_inst(insn) == 194) {
                    return SlotResult::Op(Op::RSR_EPS2);
                }
                if (field_sr_slot_inst(insn) == 195) {
                    return SlotResult::Op(Op::RSR_EPS3);
                }
                if (field_sr_slot_inst(insn) == 196) {
                    return SlotResult::Op(Op::RSR_EPS4);
                }
                if (field_sr_slot_inst(insn) == 197) {
                    return SlotResult::Op(Op::RSR_EPS5);
                }
                if (field_sr_slot_inst(insn) == 198) {
                    return SlotResult::Op(Op::RSR_EPS6);
                }
                if (field_sr_slot_inst(insn) == 199) {
                    return SlotResult::Op(Op::RSR_EPS7);
                }
                if (field_sr_slot_inst(insn) == 208) {
                    return SlotResult::Op(Op::RSR_CONFIGID1);
                }
                if (field_sr_slot_inst(insn) == 209) {
                    return SlotResult::Op(Op::RSR_EXCSAVE1);
                }
                if (field_sr_slot_inst(insn) == 210) {
                    return SlotResult::Op(Op::RSR_EXCSAVE2);
                }
                if (field_sr_slot_inst(insn) == 211) {
                    return SlotResult::Op(Op::RSR_EXCSAVE3);
                }
                if (field_sr_slot_inst(insn) == 212) {
                    return SlotResult::Op(Op::RSR_EXCSAVE4);
                }
                if (field_sr_slot_inst(insn) == 213) {
                    return SlotResult::Op(Op::RSR_EXCSAVE5);
                }
                if (field_sr_slot_inst(insn) == 214) {
                    return SlotResult::Op(Op::RSR_EXCSAVE6);
                }
                if (field_sr_slot_inst(insn) == 215) {
                    return SlotResult::Op(Op::RSR_EXCSAVE7);
                }
                if (field_sr_slot_inst(insn) == 224) {
                    return SlotResult::Op(Op::RSR_CPENABLE);
                }
                if (field_sr_slot_inst(insn) == 226) {
                    return SlotResult::Op(Op::RSR_INTERRUPT);
                }
                if (field_sr_slot_inst(insn) == 228) {
                    return SlotResult::Op(Op::RSR_INTENABLE);
                }
                if (field_sr_slot_inst(insn) == 230) {
                    return SlotResult::Op(Op::RSR_PS);
                }
                if (field_sr_slot_inst(insn) == 231) {
                    return SlotResult::Op(Op::RSR_VECBASE);
                }
                if (field_sr_slot_inst(insn) == 232) {
                    return SlotResult::Op(Op::RSR_EXCCAUSE);
                }
                if (field_sr_slot_inst(insn) == 233) {
                    return SlotResult::Op(Op::RSR_DEBUGCAUSE);
                }
                if (field_sr_slot_inst(insn) == 234) {
                    return SlotResult::Op(Op::RSR_CCOUNT);
                }
                if (field_sr_slot_inst(insn) == 235) {
                    return SlotResult::Op(Op::RSR_PRID);
                }
                if (field_sr_slot_inst(insn) == 236) {
                    return SlotResult::Op(Op::RSR_ICOUNT);
                }
                if (field_sr_slot_inst(insn) == 237) {
                    return SlotResult::Op(Op::RSR_ICOUNTLEVEL);
                }
                if (field_sr_slot_inst(insn) == 238) {
                    return SlotResult::Op(Op::RSR_EXCVADDR);
                }
                if (field_sr_slot_inst(insn) == 240) {
                    return SlotResult::Op(Op::RSR_CCOMPARE0);
                }
                if (field_sr_slot_inst(insn) == 241) {
                    return SlotResult::Op(Op::RSR_CCOMPARE1);
                }
                if (field_sr_slot_inst(insn) == 242) {
                    return SlotResult::Op(Op::RSR_CCOMPARE2);
                }
                if (field_sr_slot_inst(insn) == 244) {
                    return SlotResult::Op(Op::RSR_MISC0);
                }
                if (field_sr_slot_inst(insn) == 245) {
                    return SlotResult::Op(Op::RSR_MISC1);
                }
                if (field_sr_slot_inst(insn) == 246) {
                    return SlotResult::Op(Op::RSR_MISC2);
                }
                if (field_sr_slot_inst(insn) == 247) {
                    return SlotResult::Op(Op::RSR_MISC3);
                }
            }
            if (field_op2_slot_inst(insn) == 1) {
                if (field_sr_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::WSR_LBEG);
                }
                if (field_sr_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::WSR_LEND);
                }
                if (field_sr_slot_inst(insn) == 2) {
                    return SlotResult::Op(Op::WSR_LCOUNT);
                }
                if (field_sr_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::WSR_SAR);
                }
                if (field_sr_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::WSR_BR);
                }
                if (field_sr_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::WSR_LITBASE);
                }
                if (field_sr_slot_inst(insn) == 12) {
                    return SlotResult::Op(Op::WSR_SCOMPARE1);
                }
                if (field_sr_slot_inst(insn) == 16) {
                    return SlotResult::Op(Op::WSR_ACCLO);
                }
                if (field_sr_slot_inst(insn) == 17) {
                    return SlotResult::Op(Op::WSR_ACCHI);
                }
                if (field_sr_slot_inst(insn) == 32) {
                    return SlotResult::Op(Op::WSR_M0);
                }
                if (field_sr_slot_inst(insn) == 33) {
                    return SlotResult::Op(Op::WSR_M1);
                }
                if (field_sr_slot_inst(insn) == 34) {
                    return SlotResult::Op(Op::WSR_M2);
                }
                if (field_sr_slot_inst(insn) == 35) {
                    return SlotResult::Op(Op::WSR_M3);
                }
                if (field_sr_slot_inst(insn) == 72) {
                    return SlotResult::Op(Op::WSR_WINDOWBASE);
                }
                if (field_sr_slot_inst(insn) == 73) {
                    return SlotResult::Op(Op::WSR_WINDOWSTART);
                }
                if (field_sr_slot_inst(insn) == 89) {
                    return SlotResult::Op(Op::WSR_MMID);
                }
                if (field_sr_slot_inst(insn) == 95) {
                    return SlotResult::Op(Op::WSR_ERACCESS);
                }
                if (field_sr_slot_inst(insn) == 96) {
                    return SlotResult::Op(Op::WSR_IBREAKENABLE);
                }
                if (field_sr_slot_inst(insn) == 97) {
                    return SlotResult::Op(Op::WSR_MEMCTL);
                }
                if (field_sr_slot_inst(insn) == 99) {
                    return SlotResult::Op(Op::WSR_ATOMCTL);
                }
                if (field_sr_slot_inst(insn) == 104) {
                    return SlotResult::Op(Op::WSR_DDR);
                }
                if (field_sr_slot_inst(insn) == 128) {
                    return SlotResult::Op(Op::WSR_IBREAKA0);
                }
                if (field_sr_slot_inst(insn) == 129) {
                    return SlotResult::Op(Op::WSR_IBREAKA1);
                }
                if (field_sr_slot_inst(insn) == 144) {
                    return SlotResult::Op(Op::WSR_DBREAKA0);
                }
                if (field_sr_slot_inst(insn) == 145) {
                    return SlotResult::Op(Op::WSR_DBREAKA1);
                }
                if (field_sr_slot_inst(insn) == 160) {
                    return SlotResult::Op(Op::WSR_DBREAKC0);
                }
                if (field_sr_slot_inst(insn) == 161) {
                    return SlotResult::Op(Op::WSR_DBREAKC1);
                }
                if (field_sr_slot_inst(insn) == 176) {
                    return SlotResult::Op(Op::WSR_CONFIGID0);
                }
                if (field_sr_slot_inst(insn) == 177) {
                    return SlotResult::Op(Op::WSR_EPC1);
                }
                if (field_sr_slot_inst(insn) == 178) {
                    return SlotResult::Op(Op::WSR_EPC2);
                }
                if (field_sr_slot_inst(insn) == 179) {
                    return SlotResult::Op(Op::WSR_EPC3);
                }
                if (field_sr_slot_inst(insn) == 180) {
                    return SlotResult::Op(Op::WSR_EPC4);
                }
                if (field_sr_slot_inst(insn) == 181) {
                    return SlotResult::Op(Op::WSR_EPC5);
                }
                if (field_sr_slot_inst(insn) == 182) {
                    return SlotResult::Op(Op::WSR_EPC6);
                }
                if (field_sr_slot_inst(insn) == 183) {
                    return SlotResult::Op(Op::WSR_EPC7);
                }
                if (field_sr_slot_inst(insn) == 192) {
                    return SlotResult::Op(Op::WSR_DEPC);
                }
                if (field_sr_slot_inst(insn) == 194) {
                    return SlotResult::Op(Op::WSR_EPS2);
                }
                if (field_sr_slot_inst(insn) == 195) {
                    return SlotResult::Op(Op::WSR_EPS3);
                }
                if (field_sr_slot_inst(insn) == 196) {
                    return SlotResult::Op(Op::WSR_EPS4);
                }
                if (field_sr_slot_inst(insn) == 197) {
                    return SlotResult::Op(Op::WSR_EPS5);
                }
                if (field_sr_slot_inst(insn) == 198) {
                    return SlotResult::Op(Op::WSR_EPS6);
                }
                if (field_sr_slot_inst(insn) == 199) {
                    return SlotResult::Op(Op::WSR_EPS7);
                }
                if (field_sr_slot_inst(insn) == 209) {
                    return SlotResult::Op(Op::WSR_EXCSAVE1);
                }
                if (field_sr_slot_inst(insn) == 210) {
                    return SlotResult::Op(Op::WSR_EXCSAVE2);
                }
                if (field_sr_slot_inst(insn) == 211) {
                    return SlotResult::Op(Op::WSR_EXCSAVE3);
                }
                if (field_sr_slot_inst(insn) == 212) {
                    return SlotResult::Op(Op::WSR_EXCSAVE4);
                }
                if (field_sr_slot_inst(insn) == 213) {
                    return SlotResult::Op(Op::WSR_EXCSAVE5);
                }
                if (field_sr_slot_inst(insn) == 214) {
                    return SlotResult::Op(Op::WSR_EXCSAVE6);
                }
                if (field_sr_slot_inst(insn) == 215) {
                    return SlotResult::Op(Op::WSR_EXCSAVE7);
                }
                if (field_sr_slot_inst(insn) == 224) {
                    return SlotResult::Op(Op::WSR_CPENABLE);
                }
                if (field_sr_slot_inst(insn) == 226) {
                    return SlotResult::Op(Op::WSR_INTSET);
                }
                if (field_sr_slot_inst(insn) == 227) {
                    return SlotResult::Op(Op::WSR_INTCLEAR);
                }
                if (field_sr_slot_inst(insn) == 228) {
                    return SlotResult::Op(Op::WSR_INTENABLE);
                }
                if (field_sr_slot_inst(insn) == 230) {
                    return SlotResult::Op(Op::WSR_PS);
                }
                if (field_sr_slot_inst(insn) == 231) {
                    return SlotResult::Op(Op::WSR_VECBASE);
                }
                if (field_sr_slot_inst(insn) == 232) {
                    return SlotResult::Op(Op::WSR_EXCCAUSE);
                }
                if (field_sr_slot_inst(insn) == 233) {
                    return SlotResult::Op(Op::WSR_DEBUGCAUSE);
                }
                if (field_sr_slot_inst(insn) == 234) {
                    return SlotResult::Op(Op::WSR_CCOUNT);
                }
                if (field_sr_slot_inst(insn) == 236) {
                    return SlotResult::Op(Op::WSR_ICOUNT);
                }
                if (field_sr_slot_inst(insn) == 237) {
                    return SlotResult::Op(Op::WSR_ICOUNTLEVEL);
                }
                if (field_sr_slot_inst(insn) == 238) {
                    return SlotResult::Op(Op::WSR_EXCVADDR);
                }
                if (field_sr_slot_inst(insn) == 240) {
                    return SlotResult::Op(Op::WSR_CCOMPARE0);
                }
                if (field_sr_slot_inst(insn) == 241) {
                    return SlotResult::Op(Op::WSR_CCOMPARE1);
                }
                if (field_sr_slot_inst(insn) == 242) {
                    return SlotResult::Op(Op::WSR_CCOMPARE2);
                }
                if (field_sr_slot_inst(insn) == 244) {
                    return SlotResult::Op(Op::WSR_MISC0);
                }
                if (field_sr_slot_inst(insn) == 245) {
                    return SlotResult::Op(Op::WSR_MISC1);
                }
                if (field_sr_slot_inst(insn) == 246) {
                    return SlotResult::Op(Op::WSR_MISC2);
                }
                if (field_sr_slot_inst(insn) == 247) {
                    return SlotResult::Op(Op::WSR_MISC3);
                }
            }
            if (field_op2_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::SEXT);
            }
            if (field_op2_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::CLAMPS);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::MIN);
            }
            if (field_op2_slot_inst(insn) == 5) {
                return SlotResult::Op(Op::MAX);
            }
            if (field_op2_slot_inst(insn) == 6) {
                return SlotResult::Op(Op::MINU);
            }
            if (field_op2_slot_inst(insn) == 7) {
                return SlotResult::Op(Op::MAXU);
            }
            if (field_op2_slot_inst(insn) == 8) {
                return SlotResult::Op(Op::MOVEQZ);
            }
            if (field_op2_slot_inst(insn) == 9) {
                return SlotResult::Op(Op::MOVNEZ);
            }
            if (field_op2_slot_inst(insn) == 10) {
                return SlotResult::Op(Op::MOVLTZ);
            }
            if (field_op2_slot_inst(insn) == 11) {
                return SlotResult::Op(Op::MOVGEZ);
            }
            if (field_op2_slot_inst(insn) == 12) {
                return SlotResult::Op(Op::MOVF);
            }
            if (field_op2_slot_inst(insn) == 13) {
                return SlotResult::Op(Op::MOVT);
            }
            if (field_op2_slot_inst(insn) == 14) {
                if (field_st_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::RUR_ACCX_0);
                }
                if (field_st_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::RUR_ACCX_1);
                }
                if (field_st_slot_inst(insn) == 2) {
                    return SlotResult::Op(Op::RUR_QACC_H_0);
                }
                if (field_st_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::RUR_QACC_H_1);
                }
                if (field_st_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::RUR_QACC_H_2);
                }
                if (field_st_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::RUR_QACC_H_3);
                }
                if (field_st_slot_inst(insn) == 6) {
                    return SlotResult::Op(Op::RUR_QACC_H_4);
                }
                if (field_st_slot_inst(insn) == 7) {
                    return SlotResult::Op(Op::RUR_QACC_L_0);
                }
                if (field_st_slot_inst(insn) == 8) {
                    return SlotResult::Op(Op::RUR_QACC_L_1);
                }
                if (field_st_slot_inst(insn) == 9) {
                    return SlotResult::Op(Op::RUR_QACC_L_2);
                }
                if (field_st_slot_inst(insn) == 10) {
                    return SlotResult::Op(Op::RUR_QACC_L_3);
                }
                if (field_st_slot_inst(insn) == 11) {
                    return SlotResult::Op(Op::RUR_QACC_L_4);
                }
                if (field_st_slot_inst(insn) == 12) {
                    return SlotResult::Op(Op::RUR_GPIO_OUT);
                }
                if (field_st_slot_inst(insn) == 13) {
                    return SlotResult::Op(Op::RUR_SAR_BYTE);
                }
                if (field_st_slot_inst(insn) == 14) {
                    return SlotResult::Op(Op::RUR_FFT_BIT_WIDTH);
                }
                if (field_st_slot_inst(insn) == 15) {
                    return SlotResult::Op(Op::RUR_UA_STATE_0);
                }
                if (field_st_slot_inst(insn) == 16) {
                    return SlotResult::Op(Op::RUR_UA_STATE_1);
                }
                if (field_st_slot_inst(insn) == 17) {
                    return SlotResult::Op(Op::RUR_UA_STATE_2);
                }
                if (field_st_slot_inst(insn) == 18) {
                    return SlotResult::Op(Op::RUR_UA_STATE_3);
                }
                if (field_st_slot_inst(insn) == 231) {
                    return SlotResult::Op(Op::RUR_THREADPTR);
                }
                if (field_st_slot_inst(insn) == 232) {
                    return SlotResult::Op(Op::RUR_FCR);
                }
                if (field_st_slot_inst(insn) == 233) {
                    return SlotResult::Op(Op::RUR_FSR);
                }
            }
            if (field_op2_slot_inst(insn) == 15) {
                if (field_sr_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::WUR_ACCX_0);
                }
                if (field_sr_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::WUR_ACCX_1);
                }
                if (field_sr_slot_inst(insn) == 2) {
                    return SlotResult::Op(Op::WUR_QACC_H_0);
                }
                if (field_sr_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::WUR_QACC_H_1);
                }
                if (field_sr_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::WUR_QACC_H_2);
                }
                if (field_sr_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::WUR_QACC_H_3);
                }
                if (field_sr_slot_inst(insn) == 6) {
                    return SlotResult::Op(Op::WUR_QACC_H_4);
                }
                if (field_sr_slot_inst(insn) == 7) {
                    return SlotResult::Op(Op::WUR_QACC_L_0);
                }
                if (field_sr_slot_inst(insn) == 8) {
                    return SlotResult::Op(Op::WUR_QACC_L_1);
                }
                if (field_sr_slot_inst(insn) == 9) {
                    return SlotResult::Op(Op::WUR_QACC_L_2);
                }
                if (field_sr_slot_inst(insn) == 10) {
                    return SlotResult::Op(Op::WUR_QACC_L_3);
                }
                if (field_sr_slot_inst(insn) == 11) {
                    return SlotResult::Op(Op::WUR_QACC_L_4);
                }
                if (field_sr_slot_inst(insn) == 12) {
                    return SlotResult::Op(Op::WUR_GPIO_OUT);
                }
                if (field_sr_slot_inst(insn) == 13) {
                    return SlotResult::Op(Op::WUR_SAR_BYTE);
                }
                if (field_sr_slot_inst(insn) == 14) {
                    return SlotResult::Op(Op::WUR_FFT_BIT_WIDTH);
                }
                if (field_sr_slot_inst(insn) == 15) {
                    return SlotResult::Op(Op::WUR_UA_STATE_0);
                }
                if (field_sr_slot_inst(insn) == 16) {
                    return SlotResult::Op(Op::WUR_UA_STATE_1);
                }
                if (field_sr_slot_inst(insn) == 17) {
                    return SlotResult::Op(Op::WUR_UA_STATE_2);
                }
                if (field_sr_slot_inst(insn) == 18) {
                    return SlotResult::Op(Op::WUR_UA_STATE_3);
                }
                if (field_sr_slot_inst(insn) == 231) {
                    return SlotResult::Op(Op::WUR_THREADPTR);
                }
                if (field_sr_slot_inst(insn) == 232) {
                    return SlotResult::Op(Op::WUR_FCR);
                }
                if (field_sr_slot_inst(insn) == 233) {
                    return SlotResult::Op(Op::WUR_FSR);
                }
            }
        }
        if ((field_op1_slot_inst(insn) == 4 || field_op1_slot_inst(insn) == 5)) {
            return SlotResult::Op(Op::EXTUI);
        }
        if (field_op1_slot_inst(insn) == 8) {
            if (field_op2_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::LSX);
            }
            if (field_op2_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::LSXP);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::SSX);
            }
            if (field_op2_slot_inst(insn) == 5) {
                return SlotResult::Op(Op::SSXP);
            }
        }
        if (field_op1_slot_inst(insn) == 9) {
            if (field_op2_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::L32E);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::S32E);
            }
            if (field_op2_slot_inst(insn) == 5) {
                return SlotResult::Op(Op::S32NB);
            }
        }
        if (field_op1_slot_inst(insn) == 10) {
            if (field_op2_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::ADD_S);
            }
            if (field_op2_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::SUB_S);
            }
            if (field_op2_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::MUL_S);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::MADD_S);
            }
            if (field_op2_slot_inst(insn) == 5) {
                return SlotResult::Op(Op::MSUB_S);
            }
            if (field_op2_slot_inst(insn) == 6) {
                return SlotResult::Op(Op::MADDN_S);
            }
            if (field_op2_slot_inst(insn) == 7) {
                return SlotResult::Op(Op::DIVN_S);
            }
            if (field_op2_slot_inst(insn) == 8) {
                return SlotResult::Op(Op::ROUND_S);
            }
            if (field_op2_slot_inst(insn) == 9) {
                return SlotResult::Op(Op::TRUNC_S);
            }
            if (field_op2_slot_inst(insn) == 10) {
                return SlotResult::Op(Op::FLOOR_S);
            }
            if (field_op2_slot_inst(insn) == 11) {
                return SlotResult::Op(Op::CEIL_S);
            }
            if (field_op2_slot_inst(insn) == 12) {
                return SlotResult::Op(Op::FLOAT_S);
            }
            if (field_op2_slot_inst(insn) == 13) {
                return SlotResult::Op(Op::UFLOAT_S);
            }
            if (field_op2_slot_inst(insn) == 14) {
                return SlotResult::Op(Op::UTRUNC_S);
            }
            if (field_op2_slot_inst(insn) == 15) {
                if (field_t_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::MOV_S);
                }
                if (field_t_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::ABS_S);
                }
                if (field_t_slot_inst(insn) == 3) {
                    return SlotResult::Op(Op::CONST_S);
                }
                if (field_t_slot_inst(insn) == 4) {
                    return SlotResult::Op(Op::RFR);
                }
                if (field_t_slot_inst(insn) == 5) {
                    return SlotResult::Op(Op::WFR);
                }
                if (field_t_slot_inst(insn) == 6) {
                    return SlotResult::Op(Op::NEG_S);
                }
                if (field_t_slot_inst(insn) == 7) {
                    return SlotResult::Op(Op::DIV0_S);
                }
                if (field_t_slot_inst(insn) == 8) {
                    return SlotResult::Op(Op::RECIP0_S);
                }
                if (field_t_slot_inst(insn) == 9) {
                    return SlotResult::Op(Op::SQRT0_S);
                }
                if (field_t_slot_inst(insn) == 10) {
                    return SlotResult::Op(Op::RSQRT0_S);
                }
                if (field_t_slot_inst(insn) == 11) {
                    return SlotResult::Op(Op::NEXP01_S);
                }
                if (field_t_slot_inst(insn) == 12) {
                    return SlotResult::Op(Op::MKSADJ_S);
                }
                if (field_t_slot_inst(insn) == 13) {
                    return SlotResult::Op(Op::MKDADJ_S);
                }
                if (field_t_slot_inst(insn) == 14) {
                    return SlotResult::Op(Op::ADDEXP_S);
                }
                if (field_t_slot_inst(insn) == 15) {
                    return SlotResult::Op(Op::ADDEXPM_S);
                }
            }
        }
        if (field_op1_slot_inst(insn) == 11) {
            if (field_op2_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::UN_S);
            }
            if (field_op2_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::OEQ_S);
            }
            if (field_op2_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::UEQ_S);
            }
            if (field_op2_slot_inst(insn) == 4) {
                return SlotResult::Op(Op::OLT_S);
            }
            if (field_op2_slot_inst(insn) == 5) {
                return SlotResult::Op(Op::ULT_S);
            }
            if (field_op2_slot_inst(insn) == 6) {
                return SlotResult::Op(Op::OLE_S);
            }
            if (field_op2_slot_inst(insn) == 7) {
                return SlotResult::Op(Op::ULE_S);
            }
            if (field_op2_slot_inst(insn) == 8) {
                return SlotResult::Op(Op::MOVEQZ_S);
            }
            if (field_op2_slot_inst(insn) == 9) {
                return SlotResult::Op(Op::MOVNEZ_S);
            }
            if (field_op2_slot_inst(insn) == 10) {
                return SlotResult::Op(Op::MOVLTZ_S);
            }
            if (field_op2_slot_inst(insn) == 11) {
                return SlotResult::Op(Op::MOVGEZ_S);
            }
            if (field_op2_slot_inst(insn) == 12) {
                return SlotResult::Op(Op::MOVF_S);
            }
            if (field_op2_slot_inst(insn) == 13) {
                return SlotResult::Op(Op::MOVT_S);
            }
        }
    }
    if (field_op0_slot_inst(insn) == 1) {
        return SlotResult::Op(Op::L32R);
    }
    if (field_op0_slot_inst(insn) == 2) {
        if (field_r_slot_inst(insn) == 0) {
            return SlotResult::Op(Op::L8UI);
        }
        if (field_r_slot_inst(insn) == 1) {
            return SlotResult::Op(Op::L16UI);
        }
        if (field_r_slot_inst(insn) == 2) {
            return SlotResult::Op(Op::L32I);
        }
        if (field_r_slot_inst(insn) == 4) {
            return SlotResult::Op(Op::S8I);
        }
        if (field_r_slot_inst(insn) == 5) {
            return SlotResult::Op(Op::S16I);
        }
        if (field_r_slot_inst(insn) == 6) {
            return SlotResult::Op(Op::S32I);
        }
        if (field_r_slot_inst(insn) == 9) {
            return SlotResult::Op(Op::L16SI);
        }
        if (field_r_slot_inst(insn) == 10) {
            return SlotResult::Op(Op::MOVI);
        }
        if (field_r_slot_inst(insn) == 11) {
            return SlotResult::Op(Op::L32AI);
        }
        if (field_r_slot_inst(insn) == 12) {
            return SlotResult::Op(Op::ADDI);
        }
        if (field_r_slot_inst(insn) == 13) {
            return SlotResult::Op(Op::ADDMI);
        }
        if (field_r_slot_inst(insn) == 14) {
            return SlotResult::Op(Op::S32C1I);
        }
        if (field_r_slot_inst(insn) == 15) {
            return SlotResult::Op(Op::S32RI);
        }
    }
    if (field_op0_slot_inst(insn) == 3) {
        if (field_r_slot_inst(insn) == 0) {
            return SlotResult::Op(Op::LSI);
        }
        if (field_r_slot_inst(insn) == 4) {
            return SlotResult::Op(Op::SSI);
        }
        if (field_r_slot_inst(insn) == 8) {
            return SlotResult::Op(Op::LSIP);
        }
        if (field_r_slot_inst(insn) == 12) {
            return SlotResult::Op(Op::SSIP);
        }
    }
    if (field_op0_slot_inst(insn) == 4) {
        if (field_op2_slot_inst(insn) == 0) {
            if (field_op1_slot_inst(insn) == 8 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_LL_LDINC);
            }
            if (field_op1_slot_inst(insn) == 9 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_HL_LDINC);
            }
            if (field_op1_slot_inst(insn) == 10 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_LH_LDINC);
            }
            if (field_op1_slot_inst(insn) == 11 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_HH_LDINC);
            }
        }
        if (field_op2_slot_inst(insn) == 1) {
            if (field_op1_slot_inst(insn) == 8 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_LL_LDDEC);
            }
            if (field_op1_slot_inst(insn) == 9 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_HL_LDDEC);
            }
            if (field_op1_slot_inst(insn) == 10 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_LH_LDDEC);
            }
            if (field_op1_slot_inst(insn) == 11 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_HH_LDDEC);
            }
        }
        if (field_op2_slot_inst(insn) == 2) {
            if (field_op1_slot_inst(insn) == 4 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DD_LL);
            }
            if (field_op1_slot_inst(insn) == 5 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DD_HL);
            }
            if (field_op1_slot_inst(insn) == 6 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DD_LH);
            }
            if (field_op1_slot_inst(insn) == 7 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DD_HH);
            }
            if (field_op1_slot_inst(insn) == 8 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_LL);
            }
            if (field_op1_slot_inst(insn) == 9 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_HL);
            }
            if (field_op1_slot_inst(insn) == 10 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_LH);
            }
            if (field_op1_slot_inst(insn) == 11 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DD_HH);
            }
            if (field_op1_slot_inst(insn) == 12 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DD_LL);
            }
            if (field_op1_slot_inst(insn) == 13 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DD_HL);
            }
            if (field_op1_slot_inst(insn) == 14 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DD_LH);
            }
            if (field_op1_slot_inst(insn) == 15 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DD_HH);
            }
        }
        if (field_op2_slot_inst(insn) == 3) {
            if (field_op1_slot_inst(insn) == 4 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AD_LL);
            }
            if (field_op1_slot_inst(insn) == 5 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AD_HL);
            }
            if (field_op1_slot_inst(insn) == 6 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AD_LH);
            }
            if (field_op1_slot_inst(insn) == 7 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AD_HH);
            }
            if (field_op1_slot_inst(insn) == 8 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AD_LL);
            }
            if (field_op1_slot_inst(insn) == 9 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AD_HL);
            }
            if (field_op1_slot_inst(insn) == 10 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AD_LH);
            }
            if (field_op1_slot_inst(insn) == 11 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AD_HH);
            }
            if (field_op1_slot_inst(insn) == 12 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AD_LL);
            }
            if (field_op1_slot_inst(insn) == 13 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AD_HL);
            }
            if (field_op1_slot_inst(insn) == 14 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AD_LH);
            }
            if (field_op1_slot_inst(insn) == 15 && field_r_slot_inst(insn) == 0 && field_t3_slot_inst(insn) == 0 && field_tlo_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AD_HH);
            }
        }
        if (field_op2_slot_inst(insn) == 4) {
            if (field_op1_slot_inst(insn) == 8 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_LL_LDINC);
            }
            if (field_op1_slot_inst(insn) == 9 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_HL_LDINC);
            }
            if (field_op1_slot_inst(insn) == 10 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_LH_LDINC);
            }
            if (field_op1_slot_inst(insn) == 11 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_HH_LDINC);
            }
        }
        if (field_op2_slot_inst(insn) == 5) {
            if (field_op1_slot_inst(insn) == 8 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_LL_LDDEC);
            }
            if (field_op1_slot_inst(insn) == 9 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_HL_LDDEC);
            }
            if (field_op1_slot_inst(insn) == 10 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_LH_LDDEC);
            }
            if (field_op1_slot_inst(insn) == 11 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_HH_LDDEC);
            }
        }
        if (field_op2_slot_inst(insn) == 6) {
            if (field_op1_slot_inst(insn) == 4 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DA_LL);
            }
            if (field_op1_slot_inst(insn) == 5 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DA_HL);
            }
            if (field_op1_slot_inst(insn) == 6 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DA_LH);
            }
            if (field_op1_slot_inst(insn) == 7 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_DA_HH);
            }
            if (field_op1_slot_inst(insn) == 8 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_LL);
            }
            if (field_op1_slot_inst(insn) == 9 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_HL);
            }
            if (field_op1_slot_inst(insn) == 10 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_LH);
            }
            if (field_op1_slot_inst(insn) == 11 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_DA_HH);
            }
            if (field_op1_slot_inst(insn) == 12 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DA_LL);
            }
            if (field_op1_slot_inst(insn) == 13 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DA_HL);
            }
            if (field_op1_slot_inst(insn) == 14 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DA_LH);
            }
            if (field_op1_slot_inst(insn) == 15 && field_s_slot_inst(insn) == 0 && field_w_slot_inst(insn) == 0 && field_r3_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_DA_HH);
            }
        }
        if (field_op2_slot_inst(insn) == 7) {
            if (field_op1_slot_inst(insn) == 0 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::UMUL_AA_LL);
            }
            if (field_op1_slot_inst(insn) == 1 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::UMUL_AA_HL);
            }
            if (field_op1_slot_inst(insn) == 2 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::UMUL_AA_LH);
            }
            if (field_op1_slot_inst(insn) == 3 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::UMUL_AA_HH);
            }
            if (field_op1_slot_inst(insn) == 4 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AA_LL);
            }
            if (field_op1_slot_inst(insn) == 5 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AA_HL);
            }
            if (field_op1_slot_inst(insn) == 6 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AA_LH);
            }
            if (field_op1_slot_inst(insn) == 7 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MUL_AA_HH);
            }
            if (field_op1_slot_inst(insn) == 8 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AA_LL);
            }
            if (field_op1_slot_inst(insn) == 9 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AA_HL);
            }
            if (field_op1_slot_inst(insn) == 10 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AA_LH);
            }
            if (field_op1_slot_inst(insn) == 11 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULA_AA_HH);
            }
            if (field_op1_slot_inst(insn) == 12 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AA_LL);
            }
            if (field_op1_slot_inst(insn) == 13 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AA_HL);
            }
            if (field_op1_slot_inst(insn) == 14 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AA_LH);
            }
            if (field_op1_slot_inst(insn) == 15 && field_r_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::MULS_AA_HH);
            }
        }
        if (field_op2_slot_inst(insn) == 8) {
            if (field_op1_slot_inst(insn) == 0 && field_t_slot_inst(insn) == 0 && field_rhi_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::LDINC);
            }
        }
        if (field_op2_slot_inst(insn) == 9) {
            if (field_op1_slot_inst(insn) == 0 && field_t_slot_inst(insn) == 0 && field_rhi_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::LDDEC);
            }
        }
    }
    if (field_op0_slot_inst(insn) == 5) {
        if (field_n_slot_inst(insn) == 0) {
            return SlotResult::Op(Op::CALL0);
        }
        if (field_n_slot_inst(insn) == 1) {
            return SlotResult::Op(Op::CALL4);
        }
        if (field_n_slot_inst(insn) == 2) {
            return SlotResult::Op(Op::CALL8);
        }
        if (field_n_slot_inst(insn) == 3) {
            return SlotResult::Op(Op::CALL12);
        }
    }
    if (field_op0_slot_inst(insn) == 6) {
        if (field_n_slot_inst(insn) == 0) {
            return SlotResult::Op(Op::J);
        }
        if (field_n_slot_inst(insn) == 1) {
            if (field_m_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::BEQZ);
            }
            if (field_m_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::BNEZ);
            }
            if (field_m_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::BLTZ);
            }
            if (field_m_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::BGEZ);
            }
        }
        if (field_n_slot_inst(insn) == 2) {
            if (field_m_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::BEQI);
            }
            if (field_m_slot_inst(insn) == 1) {
                return SlotResult::Op(Op::BNEI);
            }
            if (field_m_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::BLTI);
            }
            if (field_m_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::BGEI);
            }
        }
        if (field_n_slot_inst(insn) == 3) {
            if (field_m_slot_inst(insn) == 0) {
                return SlotResult::Op(Op::ENTRY);
            }
            if (field_m_slot_inst(insn) == 1) {
                if (field_r_slot_inst(insn) == 0) {
                    return SlotResult::Op(Op::BF);
                }
                if (field_r_slot_inst(insn) == 1) {
                    return SlotResult::Op(Op::BT);
                }
                if (field_r_slot_inst(insn) == 8) {
                    return SlotResult::Op(Op::LOOP);
                }
                if (field_r_slot_inst(insn) == 9) {
                    return SlotResult::Op(Op::LOOPNEZ);
                }
                if (field_r_slot_inst(insn) == 10) {
                    return SlotResult::Op(Op::LOOPGTZ);
                }
            }
            if (field_m_slot_inst(insn) == 2) {
                return SlotResult::Op(Op::BLTUI);
            }
            if (field_m_slot_inst(insn) == 3) {
                return SlotResult::Op(Op::BGEUI);
            }
        }
    }
    if (field_op0_slot_inst(insn) == 7) {
        if (field_r_slot_inst(insn) == 0) {
            return SlotResult::Op(Op::BNONE);
        }
        if (field_r_slot_inst(insn) == 1) {
            return SlotResult::Op(Op::BEQ);
        }
        if (field_r_slot_inst(insn) == 2) {
            return SlotResult::Op(Op::BLT);
        }
        if (field_r_slot_inst(insn) == 3) {
            return SlotResult::Op(Op::BLTU);
        }
        if (field_r_slot_inst(insn) == 4) {
            return SlotResult::Op(Op::BALL);
        }
        if (field_r_slot_inst(insn) == 5) {
            return SlotResult::Op(Op::BBC);
        }
        if ((field_r_slot_inst(insn) == 6 || field_r_slot_inst(insn) == 7)) {
            return SlotResult::Op(Op::BBCI);
        }
        if (field_r_slot_inst(insn) == 8) {
            return SlotResult::Op(Op::BANY);
        }
        if (field_r_slot_inst(insn) == 9) {
            return SlotResult::Op(Op::BNE);
        }
        if (field_r_slot_inst(insn) == 10) {
            return SlotResult::Op(Op::BGE);
        }
        if (field_r_slot_inst(insn) == 11) {
            return SlotResult::Op(Op::BGEU);
        }
        if (field_r_slot_inst(insn) == 12) {
            return SlotResult::Op(Op::BNALL);
        }
        if (field_r_slot_inst(insn) == 13) {
            return SlotResult::Op(Op::BBS);
        }
        if ((field_r_slot_inst(insn) == 14 || field_r_slot_inst(insn) == 15)) {
            return SlotResult::Op(Op::BBSI);
        }
    }
    return SlotResult::Undefined;
}

pub(crate) fn slot_inst16a(insn: u32) -> SlotResult {
    if (field_op0_slot_inst16a(insn) == 8) {
        return SlotResult::Op(Op::L32I_N);
    }
    if (field_op0_slot_inst16a(insn) == 9) {
        return SlotResult::Op(Op::S32I_N);
    }
    if (field_op0_slot_inst16a(insn) == 10) {
        return SlotResult::Op(Op::ADD_N);
    }
    if (field_op0_slot_inst16a(insn) == 11) {
        return SlotResult::Op(Op::ADDI_N);
    }
    return SlotResult::Undefined;
}

pub(crate) fn slot_inst16b(insn: u32) -> SlotResult {
    if (field_op0_slot_inst16b(insn) == 12) {
        if (field_i_slot_inst16b(insn) == 0) {
            return SlotResult::Op(Op::MOVI_N);
        }
        if (field_i_slot_inst16b(insn) == 1) {
            if (field_z_slot_inst16b(insn) == 0) {
                return SlotResult::Op(Op::BEQZ_N);
            }
            if (field_z_slot_inst16b(insn) == 1) {
                return SlotResult::Op(Op::BNEZ_N);
            }
        }
    }
    if (field_op0_slot_inst16b(insn) == 13) {
        if (field_r_slot_inst16b(insn) == 0) {
            return SlotResult::Op(Op::MOV_N);
        }
        if (field_r_slot_inst16b(insn) == 15) {
            if (field_t_slot_inst16b(insn) == 0) {
                return SlotResult::Op(Op::RET_N);
            }
            if (field_t_slot_inst16b(insn) == 1) {
                return SlotResult::Op(Op::RETW_N);
            }
            if (field_t_slot_inst16b(insn) == 2) {
                return SlotResult::Op(Op::BREAK_N);
            }
            if (field_t_slot_inst16b(insn) == 3 && field_s_slot_inst16b(insn) == 0) {
                return SlotResult::Op(Op::NOP_N);
            }
            if (field_t_slot_inst16b(insn) == 6 && field_s_slot_inst16b(insn) == 0) {
                return SlotResult::Op(Op::ILL_N);
            }
        }
    }
    return SlotResult::Undefined;
}

pub(crate) fn slot_slot_format_32_0(insn: u32) -> SlotResult {
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 456 && field_fld_slot_format_32_0_7_6_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_CMUL_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 457 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VADDS_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 457 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VADDS_S32_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 457 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 2) {
        return SlotResult::Op(Op::EE_VADDS_S8_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 457 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_VMAX_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 458 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_VMUL_S8_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 458 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMAX_S32_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 458 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMIN_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 458 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 2) {
        return SlotResult::Op(Op::EE_VMIN_S8_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 459 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 2) {
        return SlotResult::Op(Op::EE_VMUL_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 459 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_VMUL_U16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 459 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMAX_S8_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 459 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMIN_S32_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 464 && field_fld_slot_format_32_0_13_11_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_SRC_Q_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 465 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VSUBS_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 465 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 2) {
        return SlotResult::Op(Op::EE_VSUBS_S32_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 465 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_VSUBS_S8_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 465 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMUL_U8_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_20_slot_slot_format_32_0(insn) == 465 && field_fld_slot_format_32_0_13_13_slot_slot_format_32_0(insn) == 0 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 4) {
        return SlotResult::Op(Op::EE_FFT_R2BF_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_22_slot_slot_format_32_0(insn) == 112 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 125) {
        return SlotResult::Op(Op::EE_LDXQ_32);
    }
    if (field_fld_slot_format_32_0_28_22_slot_slot_format_32_0(insn) == 115 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 0 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_STXQ_32);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 44) {
        return SlotResult::Op(Op::EE_VMULAS_S16_ACCX_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 45) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 46) {
        return SlotResult::Op(Op::EE_VMULAS_S8_ACCX_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 47) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 48) {
        return SlotResult::Op(Op::EE_VMULAS_U16_ACCX_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 49) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 50) {
        return SlotResult::Op(Op::EE_VMULAS_U8_ACCX_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 51) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC_LD_XP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 52) {
        return SlotResult::Op(Op::EE_FFT_AMS_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 53) {
        return SlotResult::Op(Op::EE_FFT_AMS_S16_LD_INCP_UAUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 54) {
        return SlotResult::Op(Op::EE_FFT_AMS_S16_LD_R32_DECP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 55) {
        return SlotResult::Op(Op::EE_FFT_CMUL_S16_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_5_slot_slot_format_32_0(insn) == 2) {
        return SlotResult::Op(Op::EE_LDF_64_IP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_5_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_STF_64_IP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 45) {
        return SlotResult::Op(Op::EE_VADDS_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 61) {
        return SlotResult::Op(Op::EE_VADDS_S32_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 28) {
        return SlotResult::Op(Op::EE_VADDS_S8_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 77) {
        return SlotResult::Op(Op::EE_VSUBS_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 93) {
        return SlotResult::Op(Op::EE_VSUBS_S32_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 109) {
        return SlotResult::Op(Op::EE_VSUBS_S8_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 63) {
        return SlotResult::Op(Op::EE_VMUL_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 76) {
        return SlotResult::Op(Op::EE_VMUL_S8_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 92) {
        return SlotResult::Op(Op::EE_VMUL_U16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 108) {
        return SlotResult::Op(Op::EE_VMUL_U8_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_6_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_CMUL_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 8) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC_LDBC_INCP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 9) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC_LDBC_INCP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 10) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC_LDBC_INCP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 11) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC_LDBC_INCP_QUP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 124) {
        return SlotResult::Op(Op::EE_VSMULAS_S16_QACC_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_9_slot_slot_format_32_0(insn) == 1 && field_fld_slot_format_32_0_7_4_slot_slot_format_32_0(insn) == 12) {
        return SlotResult::Op(Op::EE_VSMULAS_S8_QACC_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 29) {
        return SlotResult::Op(Op::EE_VMAX_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 30) {
        return SlotResult::Op(Op::EE_VMAX_S32_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 31) {
        return SlotResult::Op(Op::EE_VMAX_S8_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 46) {
        return SlotResult::Op(Op::EE_VMIN_S16_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 62) {
        return SlotResult::Op(Op::EE_VMIN_S32_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_10_4_slot_slot_format_32_0(insn) == 47) {
        return SlotResult::Op(Op::EE_VMIN_S8_LD_INCP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 56 && field_fld_slot_format_32_0_7_6_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_SRC_Q_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 0 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_S16_ACCX_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 2 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_S8_ACCX_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 4 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_U16_ACCX_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 6 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_U8_ACCX_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 1 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 3 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 5 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_23_slot_slot_format_32_0(insn) == 60 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 7 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC_LD_XP);
    }
    if (field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn) == 16) {
        return SlotResult::Op(Op::EE_LDF_128_IP);
    }
    if (field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn) == 17) {
        return SlotResult::Op(Op::EE_LDF_128_XP);
    }
    if (field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn) == 18) {
        return SlotResult::Op(Op::EE_STF_128_IP);
    }
    if (field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn) == 19) {
        return SlotResult::Op(Op::EE_STF_128_XP);
    }
    if (field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn) == 20) {
        return SlotResult::Op(Op::EE_FFT_AMS_S16_ST_INCP);
    }
    if (field_fld_slot_format_32_0_28_24_slot_slot_format_32_0(insn) == 21) {
        return SlotResult::Op(Op::EE_FFT_CMUL_S16_ST_XP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_S16_ACCX_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 1) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 2) {
        return SlotResult::Op(Op::EE_VMULAS_S8_ACCX_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 3) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 4) {
        return SlotResult::Op(Op::EE_VMULAS_U16_ACCX_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 5) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 6) {
        return SlotResult::Op(Op::EE_VMULAS_U8_ACCX_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 7) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC_LD_IP_QUP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 0 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_S16_ACCX_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 2 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_S8_ACCX_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 4 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_U16_ACCX_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 6 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_U8_ACCX_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 1 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_S16_QACC_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 3 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_S8_QACC_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 5 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_U16_QACC_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_25_slot_slot_format_32_0(insn) == 15 && field_fld_slot_format_32_0_19_17_slot_slot_format_32_0(insn) == 7 && field_fld_slot_format_32_0_10_8_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::EE_VMULAS_U8_QACC_LD_IP);
    }
    if (field_fld_slot_format_32_0_28_4_slot_slot_format_32_0(insn) == 30154752 && field_fld_slot_format_32_0_3_0_slot_slot_format_32_0(insn) == 0) {
        return SlotResult::Op(Op::NOP);
    }
    return SlotResult::Undefined;
}

