//! Chip-agnostic multi-precision integer arithmetic for Espressif RSA/MPI accelerators.
//!
//! Operands are little-endian slices of 32-bit limbs, up to 128 words (4096 bits). Implements
//! plain multiplication, Montgomery multiplication, two-stage modular multiplication and
//! Montgomery modular exponentiation matching the hardware's `M' = -M^-1 mod 2^32` and
//! `rb = R^2 mod M` convention.

use std::cmp::Ordering;

/// Maximum operand width in 32-bit words (4096 bits).
pub(crate) const MAX_WORDS: usize = 128;

/// Computes `Z = X * Y` where `x` and `y` have length `n` and `z` has length `2 * n`.
pub(crate) fn mul(x: &[u32], y: &[u32], z: &mut [u32]) {
    let n = x.len();
    debug_assert_eq!(y.len(), n);
    debug_assert_eq!(z.len(), 2 * n);
    z.fill(0);
    for (i, &xi) in x.iter().enumerate() {
        let xi = u64::from(xi);
        let mut carry = 0u64;
        for (j, &yj) in y.iter().enumerate() {
            let prod = xi * u64::from(yj) + u64::from(z[i + j]) + carry;
            z[i + j] = prod as u32;
            carry = prod >> 32;
        }
        z[i + n] = carry as u32;
    }
}

/// Montgomery multiplication `a * b * R^-1 mod m`, where `R = 2^(32 * n)` and
/// `m_dash = -m[0]^-1 mod 2^32`.
///
/// Uses the CIOS (Coarsely Integrated Operand Scanning) algorithm on fixed-size stack storage.
/// If `b < m` and `a < R`, the intermediate sum is strictly less than `2 * m` and a single
/// conditional subtraction produces the canonical residue in `[0, m)`. When `b >= m` (possible
/// if firmware supplies unreduced inputs), repeated reduction is avoided by full division only
/// when `t >= 2 * m`.
pub(crate) fn mont_mul(a: &[u32], b: &[u32], m: &[u32], m_dash: u32, out: &mut [u32]) {
    let n = m.len();
    debug_assert!(n > 0 && n <= MAX_WORDS);
    debug_assert_eq!(a.len(), n);
    debug_assert_eq!(b.len(), n);
    debug_assert_eq!(out.len(), n);

    let mut t = [0u32; MAX_WORDS + 2];
    for &ai in a {
        let ai = u64::from(ai);
        // t += ai * b
        let mut carry = 0u64;
        for j in 0..n {
            let sum = u64::from(t[j]) + ai * u64::from(b[j]) + carry;
            t[j] = sum as u32;
            carry = sum >> 32;
        }
        let sum = u64::from(t[n]) + carry;
        t[n] = sum as u32;
        t[n + 1] = (sum >> 32) as u32;

        // u = t[0] * m_dash mod 2^32; t = (t + u * m) >> 32
        let u = u64::from(t[0].wrapping_mul(m_dash));
        let sum0 = u64::from(t[0]) + u * u64::from(m[0]);
        let mut carry = sum0 >> 32;
        for j in 1..n {
            let sum = u64::from(t[j]) + u * u64::from(m[j]) + carry;
            t[j - 1] = sum as u32;
            carry = sum >> 32;
        }
        let sum_n = u64::from(t[n]) + carry;
        t[n - 1] = sum_n as u32;
        t[n] = t[n + 1] + (sum_n >> 32) as u32;
        t[n + 1] = 0;
    }

    // Reduce t[0..=n] modulo m when m != 0.
    if m.iter().any(|&w| w != 0) && (t[n] != 0 || cmp_limbs(&t[..n], m) != Ordering::Less) {
        sub_limbs(&mut t[..=n], m);
        // Only reachable if b >= m on entry; finish reducing cleanly.
        while t[n] != 0 || cmp_limbs(&t[..n], m) != Ordering::Less {
            rem_limbs(&mut t[..=n], m);
        }
    }
    out.copy_from_slice(&t[..n]);
}

/// Two-stage Montgomery modular multiplication `X * Y * rb * R^-2 mod M` (which equals
/// `X * Y mod M` when `rb = R^2 mod M`).
pub(crate) fn mod_mult(x: &[u32], y: &[u32], m: &[u32], rb: &[u32], m_dash: u32, out: &mut [u32]) {
    let n = m.len();
    let mut tmp = [0u32; MAX_WORDS];
    // Stage 1: tmp = MontMul(x, rb) = x * R mod m (rb < m in standard use).
    mont_mul(x, rb, m, m_dash, &mut tmp[..n]);
    // Stage 2: out = MontMul(y, tmp) = x * y mod m (put tmp as second operand since tmp < m).
    mont_mul(y, &tmp[..n], m, m_dash, out);
}

/// Montgomery modular exponentiation `X^Y mod M` using `rb = R^2 mod M` and `m_dash`.
///
/// `top_bit` is the highest bit index of `Y` to inspect (for example `search_pos` when the
/// hardware's search option is enabled, or `32 * n - 1` otherwise). Bits above `top_bit` are
/// ignored, matching the hardware's search start position.
pub(crate) fn mod_exp(
    x: &[u32],
    y: &[u32],
    m: &[u32],
    rb: &[u32],
    m_dash: u32,
    top_bit: usize,
    out: &mut [u32],
) {
    let n = m.len();
    debug_assert!(n > 0 && n <= MAX_WORDS);
    let mut one = [0u32; MAX_WORDS];
    one[0] = 1;

    let mut x_bar = [0u32; MAX_WORDS];
    let mut acc = [0u32; MAX_WORDS];
    let mut next = [0u32; MAX_WORDS];

    // x_bar = X * R mod M; acc = 1 * R mod M
    mont_mul(x, rb, m, m_dash, &mut x_bar[..n]);
    mont_mul(&one[..n], rb, m, m_dash, &mut acc[..n]);

    let limit = top_bit.min(32 * n - 1);
    if let Some(msb) = highest_set_bit(y, limit) {
        for bit in (0..=msb).rev() {
            mont_mul(&acc[..n], &acc[..n], m, m_dash, &mut next[..n]);
            acc[..n].copy_from_slice(&next[..n]);
            if get_bit(y, bit) {
                mont_mul(&acc[..n], &x_bar[..n], m, m_dash, &mut next[..n]);
                acc[..n].copy_from_slice(&next[..n]);
            }
        }
    }

    // Leave Montgomery domain: out = acc * 1 * R^-1 mod M
    mont_mul(&acc[..n], &one[..n], m, m_dash, out);
}

/// Counts the number of 1-bits in `y` at or below `limit` (inclusive).
pub(crate) fn popcount_up_to(y: &[u32], limit: usize) -> usize {
    if y.is_empty() {
        return 0;
    }
    let max_bit = limit.min(32 * y.len() - 1);
    let full_words = max_bit / 32;
    let rem_bits = max_bit % 32;
    let mut count = 0usize;
    for &word in &y[..full_words] {
        count += word.count_ones() as usize;
    }
    let mask = if rem_bits == 31 {
        u32::MAX
    } else {
        (1u32 << (rem_bits + 1)) - 1
    };
    count += (y[full_words] & mask).count_ones() as usize;
    count
}

fn get_bit(limbs: &[u32], bit: usize) -> bool {
    (limbs[bit / 32] >> (bit % 32)) & 1 != 0
}

fn highest_set_bit(limbs: &[u32], limit: usize) -> Option<usize> {
    let mut word_idx = limit / 32;
    let top_in_word = limit % 32;
    let mask = if top_in_word == 31 {
        u32::MAX
    } else {
        (1u32 << (top_in_word + 1)) - 1
    };
    let top = limbs[word_idx] & mask;
    if top != 0 {
        return Some(word_idx * 32 + (31 - top.leading_zeros() as usize));
    }
    while word_idx > 0 {
        word_idx -= 1;
        if limbs[word_idx] != 0 {
            return Some(word_idx * 32 + (31 - limbs[word_idx].leading_zeros() as usize));
        }
    }
    None
}

fn cmp_limbs(a: &[u32], b: &[u32]) -> Ordering {
    for (&x, &y) in a.iter().rev().zip(b.iter().rev()) {
        match x.cmp(&y) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    Ordering::Equal
}

/// Subtracts `m` (`n` limbs) from `t` (`n + 1` limbs), assuming `t >= m`.
fn sub_limbs(t: &mut [u32], m: &[u32]) {
    let n = m.len();
    let mut borrow = 0u64;
    for i in 0..n {
        let diff = (u64::from(t[i])).wrapping_sub(u64::from(m[i]) + borrow);
        t[i] = diff as u32;
        borrow = (diff >> 63) & 1;
    }
    t[n] = t[n].wrapping_sub(borrow as u32);
}

/// Bit-by-bit remainder `t %= m` for `t` of length `n + 1` and non-zero `m` of length `n`.
fn rem_limbs(t: &mut [u32], m: &[u32]) {
    let n = m.len();
    let mut rem = [0u32; MAX_WORDS + 1];
    let bits = 32 * (n + 1);
    for bit in (0..bits).rev() {
        // rem <<= 1
        let mut carry = u32::from(get_bit(t, bit));
        for limb in &mut rem[..=n] {
            let next_carry = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next_carry;
        }
        if rem[n] != 0 || cmp_limbs(&rem[..n], m) != Ordering::Less {
            sub_limbs(&mut rem[..=n], m);
        }
    }
    t.copy_from_slice(&rem[..=n]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Computes `M' = -M^-1 mod 2^32` for odd `M`.
    fn m_dash(m0: u32) -> u32 {
        assert_eq!(m0 & 1, 1);
        let mut inv = 1u32;
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u32.wrapping_sub(m0.wrapping_mul(inv)));
        }
        inv.wrapping_neg()
    }

    /// Computes `R^2 mod M` where `R = 2^(32 * n)`.
    fn r_squared_mod(m: &[u32]) -> Vec<u32> {
        let n = m.len();
        let mut rem = vec![0u32; n + 1];
        rem[0] = 1;
        for _ in 0..(64 * n) {
            let mut carry = 0u32;
            for limb in &mut rem[..=n] {
                let next_carry = *limb >> 31;
                *limb = (*limb << 1) | carry;
                carry = next_carry;
            }
            if rem[n] != 0 || cmp_limbs(&rem[..n], m) != Ordering::Less {
                sub_limbs(&mut rem[..=n], m);
            }
        }
        rem.truncate(n);
        rem
    }

    fn from_be_hex(hex: &str, words: usize) -> Vec<u32> {
        assert_eq!(hex.len() % 2, 0);
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut out = vec![0u32; words];
        for (i, &b) in bytes.iter().rev().enumerate() {
            out[i / 4] |= u32::from(b) << (8 * (i % 4));
        }
        out
    }

    fn to_be_hex(limbs: &[u32]) -> String {
        let mut bytes = Vec::with_capacity(limbs.len() * 4);
        for &w in limbs.iter().rev() {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        let first = bytes
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(bytes.len() - 1);
        bytes[first..].iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn plain_multiplication_matches_exact_products() {
        let x = [0xffff_ffffu32, 0x1234_5678];
        let y = [0x8765_4321u32, 0xfedc_ba98];
        let mut z = [0u32; 4];
        mul(&x, &y, &mut z);
        let xv = u128::from(x[0]) | (u128::from(x[1]) << 32);
        let yv = u128::from(y[0]) | (u128::from(y[1]) << 32);
        let want = xv * yv;
        let got = u128::from(z[0])
            | (u128::from(z[1]) << 32)
            | (u128::from(z[2]) << 64)
            | (u128::from(z[3]) << 96);
        assert_eq!(got, want);
    }

    #[test]
    fn failover_mod_mult_computes_exact_product_when_m_is_all_ones() {
        // Matches ESP-IDF's `esp_mpi_mult_mpi_failover_mod_mult_hw_op`: M = 2^(32n) - 1, M' = 1, rb = 1.
        let n = 4;
        let m = vec![u32::MAX; n];
        let mut rb = vec![0u32; n];
        rb[0] = 1;
        let x = [0xdead_beef, 0x0123_4567, 0, 0];
        let y = [0xcafe_babe, 0x89ab_cdef, 0, 0];
        let mut expected = [0u32; 4];
        mul(&x[..2], &y[..2], &mut expected);
        let mut got = [0u32; 4];
        mod_mult(&x, &y, &m, &rb, 1, &mut got);
        assert_eq!(got, expected);
    }

    /// PKCS#1 v2.1 / RSA-1024 known test vector (encrypt and decrypt round-trip).
    #[test]
    fn rsa_1024_encrypt_and_decrypt_round_trip() {
        let n_hex = "\
            a8b3b284af8eb50b387034a860f146c4919f318763cd6c5598c8ae4811a1e0ab\
            c4c7e0b082d693a5e7fced675cf4668512772c0cbc64a742c6c630f533c8cc72\
            f62ae833c40bf25842e984bb78bdbf97c0107d55bdb662f5c4e0fab9845cb514\
            8ef7392dd3aaff93ae1e6b667bb3d4247616d4f5ba10d4cfd226de88d39f16fb";
        let e_hex = "010001";
        let d_hex = "\
            53339cfdb79fc8466a655c7316aca85c55fd8f6dd898fdaf119517ef4f52e8fd\
            8e258df93fee180fa0e4ab29693cd83b152a553d4ac4d1812b8b9fa5af0e7f55\
            fe7304df41570926f3311f15c4d65a732c483116ee3d3d2d0af3549ad9bf7cbf\
            b78ad884f84d5beb04724dc7369b31def37d0cf539e9cfcdd3de653729ead5d1";
        let msg_hex = "\
            0002f8b8e143507639a79d1527d83d259029d7a74d8e1a9d8f44482e037f1f00\
            6628194e12073db03ba94cda9ef9532397d50dba79b987004afefe34";

        let words = 32;
        let m = from_be_hex(n_hex, words);
        let e = from_be_hex(e_hex, words);
        let d = from_be_hex(d_hex, words);
        let msg = from_be_hex(msg_hex, words);
        let rb = r_squared_mod(&m);
        let md = m_dash(m[0]);

        let mut ct = vec![0u32; words];
        mod_exp(&msg, &e, &m, &rb, md, 16, &mut ct);

        let mut pt = vec![0u32; words];
        mod_exp(&ct, &d, &m, &rb, md, 32 * words - 1, &mut pt);
        assert_eq!(pt, msg);
        // Verify modular multiplication consistency: ct^2 mod M == (ct * ct) mod M
        let mut sq_exp = vec![0u32; words];
        let mut two = vec![0u32; words];
        two[0] = 2;
        mod_exp(&ct, &two, &m, &rb, md, 1, &mut sq_exp);
        let mut sq_mul = vec![0u32; words];
        mod_mult(&ct, &ct, &m, &rb, md, &mut sq_mul);
        assert_eq!(to_be_hex(&sq_exp), to_be_hex(&sq_mul));
    }

    #[test]
    fn mod_exp_handles_4096_bit_operands_and_zero_exponent() {
        let words = MAX_WORDS;
        let mut m = vec![0u32; words];
        m[0] = 0xffff_ff2f;
        m[words - 1] = 0x8000_0001;
        let mut x = vec![0u32; words];
        x[0] = 5;
        let mut e = vec![0u32; words];
        e[0] = 3;
        let rb = r_squared_mod(&m);
        let md = m_dash(m[0]);
        let mut out = vec![0u32; words];
        mod_exp(&x, &e, &m, &rb, md, 2, &mut out);
        assert_eq!(out[0], 125);
        assert!(out[1..].iter().all(|&w| w == 0));

        // Zero exponent -> 1 mod M
        let zero = vec![0u32; words];
        mod_exp(&x, &zero, &m, &rb, md, 32 * words - 1, &mut out);
        assert_eq!(out[0], 1);
        assert!(out[1..].iter().all(|&w| w == 0));
    }
}
