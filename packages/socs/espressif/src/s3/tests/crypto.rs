//! Whole-chip tests for the ESP32-S3 SHA, AES, RSA and GDMA IN/interrupt peripherals:
//! block and DMA hashing, Typical and DMA-AES with descriptor writeback, RSA memory-clean timing
//! and `QUERY_IDLE` vs `CLEAR_INTERRUPT`, `WAITI` wakeups on SHA/AES/RSA/GDMA interrupts, and
//! refused settings (gating, clock change, power-down while running, reserved modes).

use super::*;

const AES: u32 = 0x6003_a000;
const SHA: u32 = 0x6003_b000;
const RSA: u32 = 0x6003_c000;
const GDMA: u32 = 0x6003_f000;
const INT_MATRIX: u32 = 0x600c_2000;
const RSA_PD_CTRL: u32 = SYSTEM + 0x40;

const SRAM_BASE: u32 = 0x3fcb_0000;

/// Enables clocks and releases resets for AES, SHA, RSA and GDMA, and powers up RSA memory.
fn enable_crypto_gates(chip: &mut Esp32s3) {
    // PERIP_CLK_EN1 (SYSTEM+0x1c): bit 1 AES, bit 2 SHA, bit 3 RSA, bit 6 GDMA
    chip.soc
        .restore(SYSTEM + 0x1c, (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6));
    // PERIP_RST_EN1 (SYSTEM+0x24): release all
    chip.soc.restore(SYSTEM + 0x24, 0);
    // SYSTEM_RSA_PD_CTRL_REG (SYSTEM+0x40): clear MEM_PD
    chip.soc.restore(RSA_PD_CTRL, 0);
}

/// Builds a program that maps `src` to CPU interrupt 1, runs `setup` stores, enables CPU
/// interrupt 1, clears `PS`, and enters `WAITI`. When the interrupt fires, the core jumps to the
/// unmapped reset vector `0x4000_0300`.
fn waiti_on_source(src: u32, setup: &[(u32, u32)]) -> Program {
    let mut lits = vec![INT_MATRIX + 4 * src];
    for &(addr, val) in setup {
        lits.push(addr);
        lits.push(val);
    }
    let mut program = Program::new(&lits);
    program.l32r(2, 0).movi(3, 1).rri8(6, 3, 2, 0);
    for i in 0..setup.len() {
        program
            .l32r(2, 1 + 2 * i)
            .l32r(3, 2 + 2 * i)
            .rri8(6, 3, 2, 0);
    }
    program
        .movi(7, 2)
        .wsr(7, INTENABLE)
        .movi(7, 0)
        .wsr(7, PS)
        .raw(&WAITI)
        .raw(&LOOP);
    program
}

#[test]
fn dma_aes_128_cbc_writes_output_descriptors_and_wakes_waiti_on_aes_and_gdma_irqs() {
    // NIST SP 800-38A F.2.1 CBC-AES128 (4 blocks = 64 bytes)
    let key: [u32; 4] = [
        u32::from_le_bytes([0x2b, 0x7e, 0x15, 0x16]),
        u32::from_le_bytes([0x28, 0xae, 0xd2, 0xa6]),
        u32::from_le_bytes([0xab, 0xf7, 0x15, 0x88]),
        u32::from_le_bytes([0x09, 0xcf, 0x4f, 0x3c]),
    ];
    let iv: [u32; 4] = [
        u32::from_le_bytes([0x00, 0x01, 0x02, 0x03]),
        u32::from_le_bytes([0x04, 0x05, 0x06, 0x07]),
        u32::from_le_bytes([0x08, 0x09, 0x0a, 0x0b]),
        u32::from_le_bytes([0x0c, 0x0d, 0x0e, 0x0f]),
    ];
    let pt: [u8; 64] = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a, 0xae, 0x2d, 0x8a, 0x57, 0x1e, 0x03, 0xac, 0x9c, 0x9e, 0xb7, 0x6f, 0xac, 0x45, 0xaf,
        0x8e, 0x51, 0x30, 0xc8, 0x1c, 0x46, 0xa3, 0x5c, 0xe4, 0x11, 0xe5, 0xfb, 0xc1, 0x19, 0x1a,
        0x0a, 0x52, 0xef, 0xf6, 0x9f, 0x24, 0x45, 0xdf, 0x4f, 0x9b, 0x17, 0xad, 0x2b, 0x41, 0x7b,
        0xe6, 0x6c, 0x37, 0x10,
    ];
    let expected_ct: [u8; 64] = [
        0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9, 0x19,
        0x7d, 0x50, 0x86, 0xcb, 0x9b, 0x50, 0x72, 0x19, 0xee, 0x95, 0xdb, 0x11, 0x3a, 0x91, 0x76,
        0x78, 0xb2, 0x73, 0xbe, 0xd6, 0xb8, 0xe3, 0xc1, 0x74, 0x3b, 0x71, 0x16, 0xe6, 0x9e, 0x22,
        0x22, 0x95, 0x16, 0x3f, 0xf1, 0xca, 0xa1, 0x68, 0x1f, 0xac, 0x09, 0x12, 0x0e, 0xca, 0x30,
        0x75, 0x86, 0xe1, 0xa7,
    ];

    let out_desc = SRAM_BASE;
    let in_desc0 = SRAM_BASE + 0x20;
    let in_desc1 = SRAM_BASE + 0x40;
    let pt_buf = SRAM_BASE + 0x100;
    let ct_buf = SRAM_BASE + 0x200;

    // Test waking on AES IRQ (source 77), GDMA IN CH0 IRQ (source 66) and GDMA OUT CH0 IRQ (71).
    for (name, src, ena_addr, ena_val) in [
        ("aes_irq", 77u32, AES + 0xb0, 1u32),
        ("gdma_in_irq", 66u32, GDMA + 0x10, 1 << 1),
        ("gdma_out_irq", 71u32, GDMA + 0x60 + 0x10, 1 << 3),
    ] {
        let mut setup = vec![
            (GDMA + 0x60 + 0x48, 6),
            (GDMA + 0x60 + 0x20, (out_desc & 0xf_ffff) | (1 << 21)),
            (GDMA + 0x48, 6),
            (GDMA + 0x20, (in_desc0 & 0xf_ffff) | (1 << 22)),
            (AES + 0x40, 0),
            (AES + 0x90, 1),
            (AES + 0x94, 1),
            (AES + 0x98, 4),
            (ena_addr, ena_val),
        ];
        for i in 0..4 {
            setup.push((AES + 4 * i as u32, key[i]));
            setup.push((AES + 0x50 + 4 * i as u32, iv[i]));
        }
        setup.push((AES + 0x48, 1)); // TRIGGER

        let program = waiti_on_source(src, &setup);
        let mut chip = program.chip();
        enable_crypto_gates(&mut chip);

        // Populate descriptors and plaintext buffer in SRAM.
        let mut out_d = Vec::new();
        out_d.extend_from_slice(&((1u32 << 31) | (1u32 << 30) | (64 << 12) | 64).to_le_bytes());
        out_d.extend_from_slice(&pt_buf.to_le_bytes());
        out_d.extend_from_slice(&0u32.to_le_bytes());
        let mut in_d0 = Vec::new();
        in_d0.extend_from_slice(&((1u32 << 31) | 32).to_le_bytes());
        in_d0.extend_from_slice(&ct_buf.to_le_bytes());
        in_d0.extend_from_slice(&in_desc1.to_le_bytes());
        let mut in_d1 = Vec::new();
        in_d1.extend_from_slice(&((1u32 << 31) | 32).to_le_bytes());
        in_d1.extend_from_slice(&(ct_buf + 32).to_le_bytes());
        in_d1.extend_from_slice(&0u32.to_le_bytes());
        chip.soc.poke(out_desc, &out_d);
        chip.soc.poke(in_desc0, &in_d0);
        chip.soc.poke(in_desc1, &in_d1);
        chip.soc.poke(pt_buf, &pt);
        chip.soc.poke(ct_buf, &[0u8; 64]);

        // 18 setup stores (~57 cycles) + 60 cycles (4 blocks * 15 APB cycles).
        // Before completion, WAITI sleeps and ciphertext buffer is still zeroes.
        assert_eq!(chip.run_until(c(80)), None, "{name}: still busy");
        assert_eq!(chip.load_word(ct_buf), Ok(0), "{name}: not yet written");

        // After completion, WAITI wakes and jumps to level-1 vector 0x4000_0300.
        let halt = chip.run_until(c(200));
        assert!(
            matches!(
                halt,
                Some(Halt::Fault(
                    _,
                    Fault::UnmappedFetch(Pc {
                        core: CoreId::Pro,
                        pc: 0x4000_0300
                    })
                ))
            ),
            "{name}: {halt:?}"
        );

        let mut got_ct = Vec::new();
        for i in 0..16 {
            got_ct.extend_from_slice(&chip.load_word(ct_buf + 4 * i).unwrap().to_le_bytes());
        }
        assert_eq!(got_ct, expected_ct, "{name}: ciphertext");

        // Descriptor writeback: owner=0, length=32 on both; suc_eof only on second.
        let dw0_0 = chip.load_word(in_desc0).unwrap();
        let dw0_1 = chip.load_word(in_desc1).unwrap();
        assert_eq!(dw0_0 & (1 << 31), 0, "{name}: desc0 owner cleared");
        assert_eq!((dw0_0 >> 12) & 0xfff, 32, "{name}: desc0 length");
        assert_eq!(dw0_0 & (1 << 30), 0, "{name}: desc0 no suc_eof");
        assert_eq!(dw0_1 & (1 << 31), 0, "{name}: desc1 owner cleared");
        assert_eq!((dw0_1 >> 12) & 0xfff, 32, "{name}: desc1 length");
        assert_ne!(dw0_1 & (1 << 30), 0, "{name}: desc1 suc_eof set");

        // GDMA IN_SUC_EOF_DES_ADDR and AES STATE == DONE (2), then DMA_EXIT(0) -> IDLE (0).
        let mut bus = chip.soc.bus();
        assert_eq!(bus.load(Cycle(200), GDMA + 0x28, Width::Word), Ok(in_desc1));
        assert_eq!(bus.load(Cycle(200), AES + 0x4c, Width::Word), Ok(2));
        bus.store(Cycle(200), AES + 0xb8, Width::Word, 0).unwrap();
        assert_eq!(bus.load(Cycle(200), AES + 0x4c, Width::Word), Ok(0));
        assert_eq!(chip.unmodeled().count(), 0, "{name}");
    }
}

#[test]
fn sha_dma_raises_interrupt_and_wakes_waiti() {
    let out_desc = SRAM_BASE;
    let msg_buf = SRAM_BASE + 0x100;

    let mut padded = [0u8; 64];
    padded[..3].copy_from_slice(b"abc");
    padded[3] = 0x80;
    padded[63] = 24;

    let setup = [
        (GDMA + 0x60 + 0x48, 7),
        (GDMA + 0x60 + 0x20, (out_desc & 0xf_ffff) | (1 << 21)),
        (SHA, 2),        // SHA-256
        (SHA + 0x0c, 1), // BLOCK_NUM = 1
        (SHA + 0x28, 1), // SHA_INT_ENA = 1
        (SHA + 0x1c, 1), // SHA_DMA_START = 1
    ];
    let mut chip = waiti_on_source(78, &setup).chip();
    enable_crypto_gates(&mut chip);

    let mut out_d = Vec::new();
    out_d.extend_from_slice(&((1u32 << 31) | (1u32 << 30) | (64 << 12) | 64).to_le_bytes());
    out_d.extend_from_slice(&msg_buf.to_le_bytes());
    out_d.extend_from_slice(&0u32.to_le_bytes());
    chip.soc.poke(out_desc, &out_d);
    chip.soc.poke(msg_buf, &padded);

    let halt = chip.run_until(c(100));
    assert!(
        matches!(
            halt,
            Some(Halt::Fault(
                _,
                Fault::UnmappedFetch(Pc {
                    core: CoreId::Pro,
                    pc: 0x4000_0300
                })
            ))
        ),
        "{halt:?}"
    );

    let mut digest = Vec::new();
    {
        let mut bus = chip.soc.bus();
        for i in 0..8 {
            let w = bus
                .load(Cycle(100), SHA + 0x40 + 4 * i, Width::Word)
                .unwrap();
            digest.extend_from_slice(&w.to_le_bytes());
        }
        // Clearing SHA IRQ drops its line.
        bus.store(Cycle(100), SHA + 0x24, Width::Word, 1).unwrap();
    }
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(chip.unmodeled().count(), 0);
}

#[test]
fn rsa_modexp_clean_wait_and_irq_wake_waiti_and_idle_stays_set_after_clear() {
    // 7^13 mod 3233 = 2861 in 1-word RSA MODEXP with RSA IRQ (source 76).
    let m: u32 = 3233;
    let m_dash: u32 = {
        let mut inv = 1u32;
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u32.wrapping_sub(m.wrapping_mul(inv)));
        }
        inv.wrapping_neg()
    };
    let r_mod_m = ((1u64 << 32) % u64::from(m)) as u32;
    let r_inv = ((u64::from(r_mod_m) * u64::from(r_mod_m)) % u64::from(m)) as u32;

    let setup = [
        (RSA, m),
        (RSA + 0x200, r_inv),
        (RSA + 0x400, 13),
        (RSA + 0x600, 7),
        (RSA + 0x800, m_dash),
        (RSA + 0x804, 0),
        (RSA + 0x820, 0),
        (RSA + 0x824, 1),
        (RSA + 0x828, 3),
        (RSA + 0x82c, 1),
        (RSA + 0x80c, 1),
    ];

    // Starting RSA before memory clean completes (104 APB cycles) is refused as `not_clean`.
    let mut chip = waiti_on_source(76, &setup).chip();
    enable_crypto_gates(&mut chip);
    assert!(matches!(
        chip.run_until(c(100)),
        Some(Halt::Fault(
            _,
            Fault::Refused(Refused::Register("RSA", "not_clean"), _)
        ))
    ));

    // Poll `RSA_QUERY_CLEAN_REG` until bit 0 sets (at cycle 104), then start MODEXP and WAITI.
    let mut lits = vec![RSA + 0x808, INT_MATRIX + 4 * 76];
    for &(addr, val) in &setup {
        lits.push(addr);
        lits.push(val);
    }
    let mut program = Program::new(&lits);
    program.l32r(2, 0);
    let poll = program.pc();
    program.rri8(2, 3, 2, 0);
    // `bbci a3, 0, poll` (op0=7, r=6, s=3, t=0)
    let imm = poll.wrapping_sub(program.pc() + 4) as u8;
    program.raw(&[0x07, 0x60 | 3, imm]);
    program.l32r(2, 1).movi(3, 1).rri8(6, 3, 2, 0);
    for i in 0..setup.len() {
        program
            .l32r(2, 2 + 2 * i)
            .l32r(3, 3 + 2 * i)
            .rri8(6, 3, 2, 0);
    }
    program
        .movi(7, 2)
        .wsr(7, INTENABLE)
        .movi(7, 0)
        .wsr(7, PS)
        .raw(&WAITI)
        .raw(&LOOP);

    let mut chip = program.chip();
    enable_crypto_gates(&mut chip);

    let halt = chip.run_until(c(300));
    assert!(
        matches!(
            halt,
            Some(Halt::Fault(
                _,
                Fault::UnmappedFetch(Pc {
                    core: CoreId::Pro,
                    pc: 0x4000_0300
                })
            ))
        ),
        "{halt:?}"
    );

    // Result in Z_MEM[0] is 2861, and QUERY_IDLE (0x818) stays 1 even after CLEAR_INTERRUPT (0x81c).
    let mut bus = chip.soc.bus();
    assert_eq!(bus.load(Cycle(300), RSA + 0x200, Width::Word), Ok(2861));
    assert_eq!(bus.load(Cycle(300), RSA + 0x818, Width::Word), Ok(1));
    bus.store(Cycle(300), RSA + 0x81c, Width::Word, 1).unwrap();
    assert_eq!(bus.load(Cycle(300), RSA + 0x818, Width::Word), Ok(1));
    assert_eq!(chip.unmodeled().count(), 0);
}

#[test]
fn gating_and_power_down_while_aes_or_rsa_runs_are_refused() {
    for (addr, block) in [(AES + 0x4c, "AES"), (RSA + 0x818, "RSA")] {
        let mut program = Program::new(&[addr]);
        program.l32r(2, 0).rri8(2, 3, 2, 0).raw(&LOOP);
        let mut chip = program.chip();
        assert_eq!(
            chip.run_until(c(50)),
            Some(Halt::Fault(
                c(1),
                Fault::Refused(
                    Refused::Register(block, "clock_off"),
                    pro(program.entry() + 3)
                )
            )),
            "{block}"
        );
    }

    // Wait for RSA clean, start MULT, then power down RSA memory while it runs -> refused.
    let mut program = Program::new(&[RSA + 0x808, RSA + 0x804, 1, RSA + 0x814, RSA_PD_CTRL]);
    program.l32r(2, 0);
    let poll = program.pc();
    program.rri8(2, 3, 2, 0);
    let imm = poll.wrapping_sub(program.pc() + 4) as u8;
    program.raw(&[0x07, 0x60 | 3, imm]);
    program
        .l32r(2, 1)
        .l32r(3, 2)
        .rri8(6, 3, 2, 0) // LENGTH = 1
        .l32r(2, 3)
        .rri8(6, 3, 2, 0) // MULT_START = 1
        .l32r(2, 4)
        .rri8(6, 3, 2, 0) // RSA_PD_CTRL = 1 while running
        .raw(&LOOP);
    let mut chip = program.chip();
    enable_crypto_gates(&mut chip);
    let halt = chip.run_until(c(250));
    assert!(
        matches!(
            halt,
            Some(Halt::Fault(
                _,
                Fault::Refused(Refused::Register("RSA", "power_down_while_running"), _)
            ))
        ),
        "{halt:?}"
    );
}
