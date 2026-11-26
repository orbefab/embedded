//! The ESP32-S3 RSA (Multi-Precision Integer) accelerator: modular exponentiation (`X^Y mod M`),
//! modular multiplication (`X*Y mod M`) and plain multiplication (`X*Y`) on 32-bit word operands
//! up to 4096 bits (2048-bit factors for plain multiplication), with memory-clean status, idle
//! status and completion interrupt (`ETS_RSA_INTR_SOURCE`, source 76).
//!
//! Important ESP32-S3 register distinction:
//! - `0x808` (`RSA_QUERY_CLEAN_REG`): `1` once internal memory initialization finishes, `0` while
//!   initializing or powered down.
//! - `0x818` (`RSA_QUERY_IDLE_REG`, aliased as `RSA_QUERY_INTERRUPT_REG` in IDF's
//!   `hwcrypto_reg.h`): `1` while idle, `0` while a calculation runs. Writing
//!   `RSA_CLEAR_INTERRUPT_REG` (`0x81c`) clears only the interrupt latch, never `0x818`.
//!
//! Timing (on the APB clock; the ESP32-S3 TRM gives no exact cycle-count table for RSA):
//! - Memory initialization after power-up / reset release: estimated 104 APB cycles (~1.3 us at
//!   80 MHz, matching IDF's `mpi_hal.c` measurement).
//! - Operations on `n`-word operands (estimated from hardware `O(n^2)` Montgomery MAC steps):
//!   - Plain multiplication (`MULT`, `n`-word inputs -> `2n`-word result): `n^2 + 2n` APB cycles.
//!   - Modular multiplication (`MOD_MULT`, 2 Montgomery stages): `2 * (n^2 + 2n)` APB cycles.
//!   - Modular exponentiation (`MODEXP`): `(3 + bits + mults) * (n^2 + 2n)` APB cycles, where
//!     `bits` is the exponent bit width scanned (honoring `SEARCH_OPEN`/`SEARCH_POS`) and `mults`
//!     is `bits` when `CONSTANT_TIME` is set or the Hamming weight of `Y[0..bits]` when clear.
//!
//! Registers come from IDF's `soc/esp32s3/include/soc/hwcrypto_reg.h` and `hal/mpi_ll.h`.

use crate::crypto::rsa::{self as math, MAX_WORDS};

use super::clock::Tick;

const M_BASE: u32 = 0x000;
const M_END: u32 = 0x200;
const Z_BASE: u32 = 0x200;
const Z_END: u32 = 0x400;
const Y_BASE: u32 = 0x400;
const Y_END: u32 = 0x600;
const X_BASE: u32 = 0x600;
const X_END: u32 = 0x800;

const M_DASH: u32 = 0x800;
const LENGTH: u32 = 0x804;
const QUERY_CLEAN: u32 = 0x808;
const MODEXP_START: u32 = 0x80c;
const MOD_MULT_START: u32 = 0x810;
const MULT_START: u32 = 0x814;
const QUERY_IDLE: u32 = 0x818;
const CLEAR_INT: u32 = 0x81c;
const CONSTANT_TIME: u32 = 0x820;
const SEARCH_OPEN: u32 = 0x824;
const SEARCH_POS: u32 = 0x828;
/// `RSA_INTERRUPT_REG` (`RSA_INT_ENA_REG`).
pub(crate) const INT_ENA: u32 = 0x82c;
const DATE: u32 = 0x830;

/// `SYSTEM_RSA_PD_CTRL_REG` offset in the SYSTEM block (`0x600c_0040`).
pub(crate) const PD_CTRL: u32 = 0x40;
const MEM_PD: u32 = 1 << 0;
const MEM_FORCE_PU: u32 = 1 << 1;
const MEM_FORCE_PD: u32 = 1 << 2;

/// Allowed bits of `RSA_INTERRUPT_REG` (`INT_ENA`).
pub(crate) const INT_ENA_ALLOWED: u32 = 1;
/// `ETS_RSA_INTR_SOURCE` in IDF's `soc/esp32s3/include/soc/interrupts.h`.
pub(crate) const SOURCE: u32 = 76;

const DATE_VALUE: u32 = 0x2019_0425;
/// Estimated APB cycles to zero the RSA memory blocks (~1.3 us at 80 MHz, per IDF `mpi_hal.c`).
const CLEAN_CYCLES: u64 = 104;

/// Whether `SYSTEM_RSA_PD_CTRL_REG` leaves the RSA memory powered up.
pub(crate) fn powered(pd_ctrl: u32) -> bool {
    pd_ctrl & MEM_FORCE_PD == 0 && (pd_ctrl & MEM_PD == 0 || pd_ctrl & MEM_FORCE_PU != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingOp {
    done_at: Tick,
    words: usize,
    result: [u32; MAX_WORDS],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rsa {
    m: [u32; MAX_WORDS],
    z: [u32; MAX_WORDS],
    y: [u32; MAX_WORDS],
    x: [u32; MAX_WORDS],
    m_dash: u32,
    length: u32,
    constant_time: bool,
    search_open: bool,
    search_pos: u32,
    int_ena: bool,
    int_raw: bool,
    /// Whether the peripheral was active (clocked, out of reset, powered up) on the last sync.
    active: bool,
    /// Whether internal memory initialization has completed.
    clean: bool,
    /// Tick when memory initialization finishes, if currently running.
    clean_at: Option<Tick>,
    pending: Option<PendingOp>,
}

impl Rsa {
    pub(crate) fn new() -> Rsa {
        Rsa {
            m: [0; MAX_WORDS],
            z: [0; MAX_WORDS],
            y: [0; MAX_WORDS],
            x: [0; MAX_WORDS],
            m_dash: 0,
            length: 0,
            constant_time: true,
            search_open: false,
            search_pos: 0,
            int_ena: false,
            int_raw: false,
            active: false,
            clean: false,
            clean_at: None,
            pending: None,
        }
    }

    /// Synchronizes clock, reset and memory power-down state at `now`.
    ///
    /// Leaving reset or powering up while clocked starts the memory-clean sequence; powering down
    /// drops `clean` and zeroes the memories.
    pub(crate) fn sync_power(&mut self, now: Tick, apb_ticks: u64, active: bool) {
        if active == self.active {
            return;
        }
        self.active = active;
        if active {
            self.m.fill(0);
            self.z.fill(0);
            self.y.fill(0);
            self.x.fill(0);
            self.clean = false;
            self.clean_at = Some(Tick(
                now.0.saturating_add(CLEAN_CYCLES.saturating_mul(apb_ticks)),
            ));
        } else {
            self.clean = false;
            self.clean_at = None;
            self.pending = None;
        }
    }

    /// Whether an RSA calculation is currently running at `now`.
    pub(crate) fn busy(&self, now: Tick) -> bool {
        self.pending.as_ref().is_some_and(|op| now < op.done_at)
    }

    /// The tick at which the active RSA operation finishes (and may assert its interrupt).
    pub(crate) fn next_event(&self) -> Option<Tick> {
        self.pending.as_ref().map(|op| op.done_at)
    }

    /// Whether the RSA interrupt source is asserted.
    pub(crate) fn line(&self) -> bool {
        self.int_raw && self.int_ena
    }

    /// Advances memory initialization and any running calculation through `now`.
    pub(crate) fn settle(&mut self, now: Tick) {
        if let Some(at) = self.clean_at
            && now >= at
        {
            self.clean = true;
            self.clean_at = None;
        }
        if let Some(op) = self.pending.take_if(|op| now >= op.done_at) {
            self.z[..op.words].copy_from_slice(&op.result[..op.words]);
            self.int_raw = true;
        }
    }

    pub(crate) fn read(&mut self, now: Tick, offset: u32) -> Option<u32> {
        self.settle(now);
        Some(match offset {
            M_BASE..M_END => self.m[((offset - M_BASE) / 4) as usize],
            Z_BASE..Z_END => self.z[((offset - Z_BASE) / 4) as usize],
            Y_BASE..Y_END => self.y[((offset - Y_BASE) / 4) as usize],
            X_BASE..X_END => self.x[((offset - X_BASE) / 4) as usize],
            M_DASH => self.m_dash,
            LENGTH => self.length,
            QUERY_CLEAN => u32::from(self.clean),
            MODEXP_START | MOD_MULT_START | MULT_START | CLEAR_INT => 0,
            QUERY_IDLE => u32::from(self.pending.is_none()),
            CONSTANT_TIME => u32::from(self.constant_time),
            SEARCH_OPEN => u32::from(self.search_open),
            SEARCH_POS => self.search_pos,
            INT_ENA => u32::from(self.int_ena),
            DATE => DATE_VALUE,
            _ => return None,
        })
    }

    pub(crate) fn write(
        &mut self,
        now: Tick,
        apb_ticks: u64,
        offset: u32,
        value: u32,
    ) -> Result<bool, &'static str> {
        self.settle(now);
        match offset {
            M_BASE..M_END => self.m[((offset - M_BASE) / 4) as usize] = value,
            Z_BASE..Z_END => self.z[((offset - Z_BASE) / 4) as usize] = value,
            Y_BASE..Y_END => self.y[((offset - Y_BASE) / 4) as usize] = value,
            X_BASE..X_END => self.x[((offset - X_BASE) / 4) as usize] = value,
            M_DASH => self.m_dash = value,
            LENGTH => self.length = value & 0x7f,
            CONSTANT_TIME => self.constant_time = value & 1 != 0,
            SEARCH_OPEN => self.search_open = value & 1 != 0,
            SEARCH_POS => self.search_pos = value & 0xfff,
            INT_ENA => self.int_ena = value & 1 != 0,
            CLEAR_INT => {
                if value & 1 != 0 {
                    self.int_raw = false;
                }
            }
            MODEXP_START | MOD_MULT_START | MULT_START if value & 1 != 0 => {
                if !self.active {
                    return Err("power_down");
                }
                if !self.clean {
                    return Err("not_clean");
                }
                if self.pending.is_some() {
                    return Ok(true);
                }
                self.start(now, apb_ticks, offset)?;
            }
            MODEXP_START | MOD_MULT_START | MULT_START | QUERY_CLEAN | QUERY_IDLE | DATE => {}
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn start(&mut self, now: Tick, apb_ticks: u64, op: u32) -> Result<(), &'static str> {
        let mut result = [0u32; MAX_WORDS];
        let (words, cycles) = match op {
            MULT_START => {
                // TRM §20.3.3: RSA_LENGTH_REG = 2*n - 1 (odd, 1..=127), X in X_MEM[0..n],
                // Y in Z_MEM[n..2n], result Z in Z_MEM[0..2n].
                let total = (self.length as usize) + 1;
                if !(2..=MAX_WORDS).contains(&total) || !total.is_multiple_of(2) {
                    return Err("length");
                }
                let n = total / 2;
                math::mul(&self.x[..n], &self.z[n..total], &mut result[..total]);
                (total, mac_cycles(n))
            }
            MOD_MULT_START => {
                let n = (self.length as usize) + 1;
                if n > MAX_WORDS {
                    return Err("length");
                }
                math::mod_mult(
                    &self.x[..n],
                    &self.y[..n],
                    &self.m[..n],
                    &self.z[..n],
                    self.m_dash,
                    &mut result[..n],
                );
                (n, 2 * mac_cycles(n))
            }
            MODEXP_START => {
                let n = (self.length as usize) + 1;
                if n > MAX_WORDS {
                    return Err("length");
                }
                let max_bit = 32 * n - 1;
                let top_bit = if self.search_open {
                    (self.search_pos as usize).min(max_bit)
                } else {
                    max_bit
                };
                math::mod_exp(
                    &self.x[..n],
                    &self.y[..n],
                    &self.m[..n],
                    &self.z[..n],
                    self.m_dash,
                    top_bit,
                    &mut result[..n],
                );
                let bits = (top_bit + 1) as u64;
                let mults = if self.constant_time {
                    bits
                } else {
                    math::popcount_up_to(&self.y[..n], top_bit) as u64
                };
                let stages = 3u64.saturating_add(bits).saturating_add(mults);
                (n, stages.saturating_mul(mac_cycles(n)))
            }
            _ => unreachable!("start op"),
        };

        let done_at = Tick(now.0.saturating_add(cycles.saturating_mul(apb_ticks)));
        self.pending = Some(PendingOp {
            done_at,
            words,
            result,
        });
        Ok(())
    }
}

/// Estimated APB cycles for one `n`-word Montgomery multiplication or plain multiplication.
fn mac_cycles(n: usize) -> u64 {
    let n = n as u64;
    n * n + 2 * n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_status_idle_status_and_interrupt_latch_behave_as_documented() {
        let mut rsa = Rsa::new();
        let apb = 6;
        assert_eq!(rsa.read(Tick(0), QUERY_CLEAN), Some(0));
        rsa.sync_power(Tick(10), apb, true);
        assert_eq!(rsa.read(Tick(10), QUERY_CLEAN), Some(0));
        assert_eq!(
            rsa.write(Tick(10), apb, MULT_START, 1),
            Err("not_clean"),
            "refused while memory init is in flight"
        );
        let cleaned = Tick(10 + CLEAN_CYCLES * apb);
        assert_eq!(rsa.read(Tick(cleaned.0 - 1), QUERY_CLEAN), Some(0));
        assert_eq!(rsa.read(cleaned, QUERY_CLEAN), Some(1));
        assert_eq!(rsa.read(cleaned, QUERY_IDLE), Some(1));

        // 1-word plain multiply: 7 * 11 = 77 (X[0] = 7, Z[1] = 11, LENGTH = 1 -> 2 words)
        rsa.write(cleaned, apb, X_BASE, 7).unwrap();
        rsa.write(cleaned, apb, Z_BASE + 4, 11).unwrap();
        rsa.write(cleaned, apb, LENGTH, 1).unwrap();
        rsa.write(cleaned, apb, INT_ENA, 1).unwrap();
        rsa.write(cleaned, apb, MULT_START, 1).unwrap();

        assert_eq!(rsa.read(cleaned, QUERY_IDLE), Some(0));
        assert!(!rsa.line());

        let done = rsa.next_event().unwrap();
        assert_eq!(rsa.read(done, QUERY_IDLE), Some(1));
        assert!(rsa.line());
        assert_eq!(rsa.read(done, Z_BASE), Some(77));
        assert_eq!(rsa.read(done, Z_BASE + 4), Some(0));

        // Clearing the interrupt drops the IRQ line while QUERY_IDLE stays 1 (matching ESP32-S3).
        rsa.write(done, apb, CLEAR_INT, 1).unwrap();
        assert!(!rsa.line());
        assert_eq!(rsa.read(done, QUERY_IDLE), Some(1));
    }

    #[test]
    fn odd_mult_length_and_power_down_are_refused() {
        let mut rsa = Rsa::new();
        assert_eq!(rsa.write(Tick(0), 6, MULT_START, 1), Err("power_down"));
        rsa.sync_power(Tick(0), 6, true);
        let ready = Tick(CLEAN_CYCLES * 6);
        assert_eq!(rsa.read(ready, QUERY_CLEAN), Some(1));
        // LENGTH = 0 -> total = 1 (odd, invalid for MULT)
        rsa.write(ready, 6, LENGTH, 0).unwrap();
        assert_eq!(rsa.write(ready, 6, MULT_START, 1), Err("length"));
    }
}
