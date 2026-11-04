//! Single-precision floating point on bit patterns, as both chips' FPU computes it. Every rule
//! here is integer arithmetic, so a result is the same on every host and in wasm.
//!
//! Each operation returns the result bits and the `FSR` flags it raises. The core ORs the flags
//! into `FSR`. No enable bit traps.
//!
//! The rules come from the classic and the S3 `fpu` fixtures, which agree row for row:
//!
//! - A NaN operand gives the first NaN in the op's order, quieted. Any signaling NaN raises
//!   invalid. `inf - inf` and `0 * inf` give the default NaN `7fc00000` with invalid.
//! - Arithmetic rounds once, in the mode `FCR` names. Underflow is raised only when the result is
//!   also inexact, with tininess detected after rounding as the ISA defines it (4.3.11.3, p 109).
//!   No fixture row tells that from tininess before rounding, nor reaches the sign of an exact
//!   zero sum, which follows IEEE 754.
//! - The seed ops `recip0.s` and `div0.s` read an 8-bit estimate from the reciprocal table the
//!   ISA prints (4.3.11.5, p 114). `sqrt0.s` and `rsqrt0.s` read the square-root table (p 115).
//!   See [`RECIPROCAL`] and [`RSQRT`].
//! - The divide and square-root steps `mkdadj.s`, `mksadj.s`, `addexp.s`, `addexpm.s`, `nexp01.s`
//!   and `divn.s` carry a scale in an exponent field, so a quotient's final exponent can leave
//!   the 8-bit range until `divn.s` rounds it.

pub(crate) const INEXACT: u32 = 0x80;
pub(crate) const UNDERFLOW: u32 = 0x100;
pub(crate) const OVERFLOW: u32 = 0x200;
pub(crate) const DIVIDE_BY_ZERO: u32 = 0x400;
pub(crate) const INVALID: u32 = 0x800;

const SIGN: u32 = 0x8000_0000;
const QUIET: u32 = 0x40_0000;
const FRACTION: u32 = 0x7f_ffff;
const INF: u32 = 0x7f80_0000;
const DEFAULT_NAN: u32 = 0x7fc0_0000;
const ONE: u32 = 0x3f80_0000;

/// Result bits and the flags raised.
pub(crate) type Out = (u32, u32);

/// `FCR` bits 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Round {
    Nearest,
    Zero,
    Up,
    Down,
}

impl Round {
    pub(crate) fn of_fcr(fcr: u32) -> Round {
        match fcr & 3 {
            0 => Round::Nearest,
            1 => Round::Zero,
            2 => Round::Up,
            _ => Round::Down,
        }
    }
}

/// How `trunc.s`, `round.s`, `floor.s` and `ceil.s` pick the integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToInt {
    Trunc,
    Nearest,
    Floor,
    Ceil,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Compare {
    Oeq,
    Olt,
    Ole,
    Ueq,
    Ult,
    Ule,
    Un,
}

fn is_nan(b: u32) -> bool {
    b & INF == INF && b & FRACTION != 0
}

fn is_signaling(b: u32) -> bool {
    is_nan(b) && b & QUIET == 0
}

fn is_inf(b: u32) -> bool {
    b & !SIGN == INF
}

fn is_zero(b: u32) -> bool {
    b & !SIGN == 0
}

fn negative(b: u32) -> bool {
    b & SIGN != 0
}

fn exponent(b: u32) -> i32 {
    ((b >> 23) & 0xff) as i32
}

fn sign_of(neg: bool) -> u32 {
    if neg { SIGN } else { 0 }
}

/// `mant * 2^exp`, plus a part strictly between 0 and `2^exp` when `sticky` is set. A sticky
/// value always has at least two bits below a single-precision result's last place.
#[derive(Clone, Copy, Debug)]
struct Exact {
    neg: bool,
    mant: u128,
    exp: i32,
    sticky: bool,
}

impl Exact {
    /// A finite operand.
    fn of(b: u32) -> Exact {
        let (mant, exp) = match exponent(b) {
            0 => (b & FRACTION, -149),
            e => (b & FRACTION | 0x80_0000, e - 150),
        };
        Exact {
            neg: negative(b),
            mant: u128::from(mant),
            exp,
            sticky: false,
        }
    }

    fn integer(neg: bool, mant: u32, exp: i32) -> Exact {
        Exact {
            neg,
            mant: u128::from(mant),
            exp,
            sticky: false,
        }
    }

    fn times(self, other: Exact) -> Exact {
        Exact {
            neg: self.neg != other.neg,
            mant: self.mant * other.mant,
            exp: self.exp + other.exp,
            sticky: false,
        }
    }

    fn is_zero(self) -> bool {
        self.mant == 0 && !self.sticky
    }
}

/// The exact sum of two values of at most 49 bits each. When their exponents are more than 76
/// apart, the smaller one is less than half a unit below the larger shifted up by 26 bits, so it
/// only decides the sticky part.
fn sum(x: Exact, y: Exact) -> Exact {
    if y.mant == 0 {
        return x;
    }
    if x.mant == 0 {
        return y;
    }
    let (hi, lo) = if x.exp >= y.exp { (x, y) } else { (y, x) };
    let gap = hi.exp.abs_diff(lo.exp);
    if gap <= 76 {
        let big = hi.mant << gap;
        let (neg, mant) = if hi.neg == lo.neg {
            (hi.neg, big + lo.mant)
        } else if big >= lo.mant {
            (hi.neg, big - lo.mant)
        } else {
            (lo.neg, lo.mant - big)
        };
        return Exact {
            neg,
            mant,
            exp: lo.exp,
            sticky: false,
        };
    }
    let big = hi.mant << 26;
    Exact {
        neg: hi.neg,
        mant: if hi.neg == lo.neg { big } else { big - 1 },
        exp: hi.exp - 26,
        sticky: true,
    }
}

/// `x` rounded to a multiple of `2^lsb`, as that multiple, and whether it was inexact.
fn scale(x: Exact, lsb: i32, rm: Round) -> (u128, bool) {
    let shift = lsb - x.exp;
    if shift <= 0 {
        return (x.mant << shift.unsigned_abs(), x.sticky);
    }
    let shift = shift.unsigned_abs();
    // Every value here is below 2^127, so a shift of 128 or more leaves less than half a unit.
    let (n, rem, half) = if shift >= 128 {
        (0, x.mant, None)
    } else {
        (
            x.mant >> shift,
            x.mant & ((1 << shift) - 1),
            Some(1u128 << (shift - 1)),
        )
    };
    let inexact = rem != 0 || x.sticky;
    let above = half.is_some_and(|h| rem > h || (rem == h && x.sticky));
    let tie = half.is_some_and(|h| rem == h && !x.sticky);
    let up = match rm {
        Round::Nearest => above || (tie && n & 1 == 1),
        Round::Zero => false,
        Round::Up => inexact && !x.neg,
        Round::Down => inexact && x.neg,
    };
    (n + u128::from(up), inexact)
}

/// Rounds a nonzero value to single precision. A value below `2^-126` rounds to a multiple of
/// `2^-149`. Tininess is decided after rounding, on the value rounded to 24 bits with an unbounded
/// exponent, and underflow is raised only when the result is also inexact.
fn round(x: Exact, rm: Round) -> Out {
    let sign = sign_of(x.neg);
    let top = x.exp + 127 - x.mant.leading_zeros() as i32;
    let (n, inexact) = scale(x, top - 23, rm);
    let carried = n >> 24 != 0;
    let rounded_top = top + i32::from(carried);
    if top < -126 {
        let (n, inexact) = scale(x, -149, rm);
        let flags = match (inexact, rounded_top < -126) {
            (false, _) => 0,
            (true, true) => INEXACT | UNDERFLOW,
            (true, false) => INEXACT,
        };
        return (sign | n as u32, flags);
    }
    if rounded_top + 127 >= 255 {
        let to_inf = match rm {
            Round::Nearest => true,
            Round::Zero => false,
            Round::Up => !x.neg,
            Round::Down => x.neg,
        };
        let bits = if to_inf { INF } else { INF - 1 };
        return (sign | bits, OVERFLOW | INEXACT);
    }
    let n = if carried { n >> 1 } else { n };
    let field = (rounded_top + 127) as u32;
    let flags = if inexact { INEXACT } else { 0 };
    (sign | field << 23 | (n as u32 & FRACTION), flags)
}

/// Rounds `x`, or gives the zero of sign `zero_neg` when it is exactly zero.
fn finish(x: Exact, rm: Round, zero_neg: bool) -> Out {
    if x.is_zero() {
        (sign_of(zero_neg), 0)
    } else {
        round(x, rm)
    }
}

fn first_nan(ops: &[u32]) -> Option<Out> {
    let flags = if ops.iter().any(|&b| is_signaling(b)) {
        INVALID
    } else {
        0
    };
    ops.iter()
        .find(|&&b| is_nan(b))
        .map(|&b| (b | QUIET, flags))
}

pub(crate) fn add(a: u32, b: u32, rm: Round) -> Out {
    if let Some(nan) = first_nan(&[a, b]) {
        return nan;
    }
    if is_inf(a) && is_inf(b) && a != b {
        return (DEFAULT_NAN, INVALID);
    }
    if is_inf(a) {
        return (a, 0);
    }
    if is_inf(b) {
        return (b, 0);
    }
    let zero_neg = if is_zero(a) && is_zero(b) && a == b {
        negative(a)
    } else {
        rm == Round::Down
    };
    finish(sum(Exact::of(a), Exact::of(b)), rm, zero_neg)
}

pub(crate) fn sub(a: u32, b: u32, rm: Round) -> Out {
    add(a, if is_nan(b) { b } else { b ^ SIGN }, rm)
}

pub(crate) fn mul(a: u32, b: u32, rm: Round) -> Out {
    if let Some(nan) = first_nan(&[a, b]) {
        return nan;
    }
    let neg = negative(a) != negative(b);
    if (is_inf(a) && is_zero(b)) || (is_zero(a) && is_inf(b)) {
        return (DEFAULT_NAN, INVALID);
    }
    if is_inf(a) || is_inf(b) {
        return (sign_of(neg) | INF, 0);
    }
    finish(Exact::of(a).times(Exact::of(b)), rm, neg)
}

/// `c + a * b`, or `c - a * b` when `negate` is set, with one rounding. The NaN order is `c`, `a`,
/// `b`.
pub(crate) fn fused(c: u32, a: u32, b: u32, negate: bool, rm: Round) -> Out {
    if let Some(nan) = first_nan(&[c, a, b]) {
        return nan;
    }
    let product_neg = (negative(a) != negative(b)) != negate;
    if (is_inf(a) && is_zero(b)) || (is_zero(a) && is_inf(b)) {
        return (DEFAULT_NAN, INVALID);
    }
    if is_inf(a) || is_inf(b) {
        if is_inf(c) && negative(c) != product_neg {
            return (DEFAULT_NAN, INVALID);
        }
        return (sign_of(product_neg) | INF, 0);
    }
    if is_inf(c) {
        return (c, 0);
    }
    let mut product = Exact::of(a).times(Exact::of(b));
    product.neg = product_neg;
    let zero_neg = if is_zero(c) && product.mant == 0 && negative(c) == product_neg {
        product_neg
    } else {
        rm == Round::Down
    };
    finish(sum(Exact::of(c), product), rm, zero_neg)
}

/// `maddn.s` rounds to nearest whatever `FCR` holds, and raises no flag.
pub(crate) fn maddn(c: u32, a: u32, b: u32) -> u32 {
    fused(c, a, b, false, Round::Nearest).0
}

/// `float.s` and `ufloat.s`: `a / 2^n`.
pub(crate) fn from_int(a: u32, n: u32, signed: bool, rm: Round) -> Out {
    let neg = signed && (a as i32) < 0;
    let mag = if neg { a.wrapping_neg() } else { a };
    finish(Exact::integer(neg, mag, -(n as i32)), rm, false)
}

/// The magnitude of a finite `b * 2^n` picked by `mode`, saturated at `2^33`, and whether it was
/// inexact.
fn magnitude(b: u32, n: u32, mode: ToInt) -> (u64, bool) {
    let x = Exact::of(b);
    let mant = x.mant as u64;
    let shift = x.exp + n as i32;
    if shift >= 0 {
        if mant != 0 && shift > 33 {
            return (1 << 33, false);
        }
        return ((mant << shift).min(1 << 33), false);
    }
    let down = shift.unsigned_abs();
    let (whole, rem, half) = if down >= 64 {
        (0, mant, u64::MAX)
    } else {
        (mant >> down, mant & ((1 << down) - 1), 1 << (down - 1))
    };
    let up = match mode {
        ToInt::Trunc => false,
        ToInt::Nearest => rem > half || (rem == half && whole & 1 == 1),
        ToInt::Floor => x.neg && rem != 0,
        ToInt::Ceil => !x.neg && rem != 0,
    };
    (whole + u64::from(up), rem != 0)
}

/// `trunc.s`, `round.s`, `floor.s` and `ceil.s`: `a * 2^n` as a signed word. A NaN, an infinity
/// or a value out of range saturates and raises invalid.
pub(crate) fn to_signed(a: u32, n: u32, mode: ToInt) -> Out {
    if is_nan(a) {
        return (i32::MAX as u32, INVALID);
    }
    if is_inf(a) {
        let bits = if negative(a) { i32::MIN } else { i32::MAX };
        return (bits as u32, INVALID);
    }
    let (mag, inexact) = magnitude(a, n, mode);
    let value = if negative(a) {
        -(mag as i64)
    } else {
        mag as i64
    };
    if value < i64::from(i32::MIN) {
        return (i32::MIN as u32, INVALID);
    }
    if value > i64::from(i32::MAX) {
        return (i32::MAX as u32, INVALID);
    }
    (value as u32, if inexact { INEXACT } else { 0 })
}

/// `utrunc.s`. A negative value that truncates to a nonzero integer gives that integer's two's
/// complement with invalid, and one at or below `-2^31` gives `80000000`.
pub(crate) fn to_unsigned(a: u32, n: u32) -> Out {
    if negative(a) && !is_nan(a) {
        if is_inf(a) {
            return (SIGN, INVALID);
        }
        let (mag, inexact) = magnitude(a, n, ToInt::Trunc);
        if mag == 0 {
            return (0, if inexact { INEXACT } else { 0 });
        }
        if mag > 1 << 31 {
            return (SIGN, INVALID);
        }
        return ((mag as u32).wrapping_neg(), INVALID);
    }
    if is_nan(a) || is_inf(a) {
        return (u32::MAX, INVALID);
    }
    let (mag, inexact) = magnitude(a, n, ToInt::Trunc);
    if mag > u64::from(u32::MAX) {
        return (u32::MAX, INVALID);
    }
    (mag as u32, if inexact { INEXACT } else { 0 })
}

/// The compare ops. `olt.s` and `ole.s` raise invalid on any NaN, the others only on a
/// signaling one. The `u` forms are true when the operands are unordered.
pub(crate) fn compare(a: u32, b: u32, kind: Compare) -> (bool, u32) {
    let unordered = is_nan(a) || is_nan(b);
    let signals = matches!(kind, Compare::Olt | Compare::Ole);
    let flags = if is_signaling(a) || is_signaling(b) || (unordered && signals) {
        INVALID
    } else {
        0
    };
    if unordered {
        return (
            matches!(
                kind,
                Compare::Ueq | Compare::Ult | Compare::Ule | Compare::Un
            ),
            flags,
        );
    }
    let key = |b: u32| {
        let mag = i64::from(b & !SIGN);
        if negative(b) { -mag } else { mag }
    };
    let (x, y) = (key(a), key(b));
    let result = match kind {
        Compare::Oeq | Compare::Ueq => x == y,
        Compare::Olt | Compare::Ult => x < y,
        Compare::Ole | Compare::Ule => x <= y,
        Compare::Un => false,
    };
    (result, flags)
}

/// `const.s`: 0, 1, 2 and 0.5 by the low two bits of the immediate.
pub(crate) fn constant(n: u32) -> u32 {
    [0, ONE, 0x4000_0000, 0x3f00_0000][(n & 3) as usize]
}

/// A nonzero finite operand's unbiased exponent and fraction, a subnormal one normalized.
fn normalized(b: u32) -> (i32, u32) {
    match exponent(b) {
        0 => {
            let fraction = b & FRACTION;
            let shift = fraction.leading_zeros() as i32 - 8;
            (-126 - shift, (fraction << shift) & FRACTION)
        }
        e => (e - 127, b & FRACTION),
    }
}

/// `nexp01.s`: the fraction with an exponent of 0 or 1, of the parity of the operand's own, and
/// the sign flipped.
pub(crate) fn nexp01(a: u32) -> u32 {
    if is_nan(a) {
        return a ^ SIGN;
    }
    let flipped = sign_of(!negative(a));
    if is_inf(a) || is_zero(a) {
        return flipped | ONE;
    }
    let (e, fraction) = normalized(a);
    let field = (e - 2 * (e >> 1) + 127) as u32;
    flipped | field << 23 | fraction
}

/// `addexp.s fr, fs`: `fs`'s exponent added to `fr`'s, and the signs multiplied.
pub(crate) fn addexp(r: u32, s: u32) -> u32 {
    let field = (exponent(r) + exponent(s) - 127) as u32 & 0xff;
    ((r ^ s) & SIGN) | field << 23 | (r & FRACTION)
}

/// `addexpm.s fr, fs`: the scale `mkdadj.s` or `mksadj.s` left in `fs` bits 14 to 21 added to
/// `fr`'s exponent, and `fs` bit 22 flipping the sign.
pub(crate) fn addexpm(r: u32, s: u32) -> u32 {
    let field = (exponent(r) + ((s >> 14) & 0xff) as i32 - 127) as u32 & 0xff;
    ((r & SIGN) ^ ((s << 9) & SIGN)) | field << 23 | (r & FRACTION)
}

/// The scale word of `mkdadj.s` and `mksadj.s`. Bits 23 to 30 hold the low five bits of `scale`
/// as `127 + 8 * low`, and bits 14 to 21 the rest as `127 + 8 * high`.
fn adjust(scale: i32) -> u32 {
    let low = (127 + 8 * scale.rem_euclid(32)) as u32 & 0xff;
    let high = (127 + 8 * scale.div_euclid(32)) as u32 & 0xff;
    low << 23 | high << 14
}

/// A special class in the scale word: 0 for +0, 1 for -0, 2 for +inf, 3 for -inf, and 4 or 5
/// for a NaN of either sign. `divn.s` reads the class back.
fn special(class: u32) -> u32 {
    adjust(384 + class as i32)
}

/// `mksadj.s`: the scale for a square root of `a`.
pub(crate) fn mksadj(a: u32) -> Out {
    if is_nan(a) {
        return (special(4), if is_signaling(a) { INVALID } else { 0 });
    }
    if is_zero(a) {
        return (special(u32::from(negative(a))), 0);
    }
    if negative(a) {
        return (special(4), INVALID);
    }
    if is_inf(a) {
        return (special(2), 0);
    }
    (adjust(normalized(a).0 >> 1), 0)
}

/// `mkdadj.s fr, fs`: the scale for `fs / fr`.
pub(crate) fn mkdadj(divisor: u32, dividend: u32) -> Out {
    let neg = u32::from(negative(dividend) != negative(divisor));
    if is_nan(dividend) || is_nan(divisor) {
        let flags = if is_signaling(dividend) || is_signaling(divisor) {
            INVALID
        } else {
            0
        };
        return (special(4 + neg), flags);
    }
    if (is_zero(dividend) && is_zero(divisor)) || (is_inf(dividend) && is_inf(divisor)) {
        return (special(4 + neg), INVALID);
    }
    if is_inf(dividend) {
        return (special(2 + neg), 0);
    }
    if is_zero(divisor) {
        return (special(2 + neg), DIVIDE_BY_ZERO);
    }
    if is_zero(dividend) || is_inf(divisor) {
        return (special(neg), 0);
    }
    let scale = 2 * ((normalized(dividend).0 >> 1) - (normalized(divisor).0 >> 1));
    (adjust(scale), 0)
}

/// `divn.s fr, fs, ft`: `fr + fs * ft`, scaled by the scale `fr` and `ft` carry, rounded once.
///
/// `fr` holds the quotient estimate. Its exponent is `8 * high + e` with `e` from -3 to 4, and
/// `high` from 13 up stands for `high - 32`. `ft`'s exponent holds the scale's low five bits the
/// same way. `fs` is the remainder with its own exponent. A nonzero fraction counts the implicit
/// bit.
///
/// A zero remainder adds nothing, so an exact quotient comes out exact. A NaN remainder passes
/// through quieted, with the flags of the scaled estimate alone. An exact zero sum gives a zero
/// fraction whose exponent field is `e + scale + 77`, the one zero row in the fixture, and
/// negative only when rounding down. A `high` of 12 is a special class from
/// `mkdadj.s`, which no fixture row reaches. This core gives that class's value with no flag.
pub(crate) fn divn(q: u32, r: u32, y: u32, rm: Round) -> Out {
    let tq = exponent(q) - 127;
    let mut high = (tq + 3) >> 3;
    let qe = tq - 8 * high;
    if high >= 13 {
        high -= 32;
    }
    let ty = exponent(y) - 127;
    let low = (ty + 3) >> 3;
    let ye = ty - 8 * low;
    let scale = 32 * high + (low & 31);
    let implicit = |b: u32, e: i32| Exact::integer(negative(b), b & FRACTION | 0x80_0000, e - 23);
    let estimate = implicit(q, qe + scale);
    if is_nan(r) {
        return (r | QUIET, round(estimate, rm).1);
    }
    if high == 12 {
        let class = (low & 31) as u32;
        let bits = match class {
            0 => 0,
            1 => SIGN,
            2 => INF,
            3 => SIGN | INF,
            _ => DEFAULT_NAN | (class & 1) << 31,
        };
        return (bits, 0);
    }
    if is_zero(r) {
        return round(estimate, rm);
    }
    let product = implicit(r, exponent(r) - 127).times(implicit(y, ye + scale));
    let total = sum(estimate, product);
    if total.is_zero() {
        let field = (qe + scale + 127 - 50) as u32 & 0xff;
        return (sign_of(rm == Round::Down) | field << 23, 0);
    }
    round(total, rm)
}

/// The reciprocal estimates the ISA prints for `recip0.s` and `div0.s` (4.3.11.5, p 114),
/// indexed by the top seven fraction bits, in the printed rows.
#[rustfmt::skip]
const RECIPROCAL: [u8; 128] = [
    255, 253, 251, 249, 247, 245, 244, 242, 240, 238, 237, 235, 233, 232, 230, 228,
    227, 225, 224, 222, 221, 219, 218, 216, 215, 213, 212, 211, 209, 208, 207, 205,
    204, 203, 202, 200, 199, 198, 197, 196, 194, 193, 192, 191, 190, 189, 188, 187,
    186, 185, 184, 183, 182, 181, 180, 179, 178, 177, 176, 175, 174, 173, 172, 171,
    170, 169, 168, 168, 167, 166, 165, 164, 163, 163, 162, 161, 160, 159, 159, 158,
    157, 156, 156, 155, 154, 153, 153, 152, 151, 151, 150, 149, 149, 148, 147, 147,
    146, 145, 145, 144, 143, 143, 142, 142, 141, 140, 140, 139, 139, 138, 137, 137,
    136, 136, 135, 135, 134, 133, 133, 132, 132, 131, 131, 130, 130, 129, 129, 129,
];

/// The reciprocal square-root estimates the ISA prints for `sqrt0.s` and `rsqrt0.s` (4.3.11.5,
/// p 115), in the printed rows. The index is the low bit of the biased exponent, then the top
/// six fraction bits.
#[rustfmt::skip]
const RSQRT: [u8; 128] = [
    180, 179, 178, 176, 175, 174, 172, 171, 170, 169, 168, 167, 166, 165, 163, 162,
    161, 160, 159, 158, 158, 157, 156, 155, 154, 153, 152, 151, 151, 150, 149, 148,
    147, 147, 146, 145, 144, 144, 143, 142, 142, 141, 140, 140, 139, 138, 138, 137,
    137, 136, 135, 135, 134, 134, 133, 132, 132, 131, 131, 130, 130, 129, 129, 128,
    255, 253, 251, 249, 247, 246, 244, 242, 241, 239, 237, 236, 234, 233, 231, 230,
    228, 227, 225, 224, 223, 221, 220, 219, 218, 216, 215, 214, 213, 212, 211, 210,
    208, 207, 206, 205, 204, 203, 202, 201, 200, 199, 198, 198, 197, 196, 195, 194,
    193, 192, 191, 191, 190, 189, 188, 187, 187, 186, 185, 184, 184, 183, 182, 181,
];

fn reciprocal(fraction: u32) -> u8 {
    RECIPROCAL[(fraction >> 16) as usize]
}

/// The estimate for unbiased exponent `e`. A zero reads as -127 and an infinity or a NaN as 128,
/// their biased exponents 0 and 255 less the bias.
fn rsqrt(e: i32, fraction: u32) -> u8 {
    RSQRT[((e + 127) & 1) as usize * 64 + (fraction >> 17) as usize]
}

/// `estimate * 2^shift` rounded to nearest, with no flag.
fn seed(neg: bool, estimate: u8, shift: i32) -> u32 {
    round(
        Exact::integer(neg, u32::from(estimate), shift),
        Round::Nearest,
    )
    .0
}

/// The exponent and fraction `div0.s` and `sqrt0.s` read: a zero as exponent `zero`, an infinity
/// or a NaN as exponent 128 with its fraction.
fn seed_input(a: u32, zero: i32) -> (i32, u32) {
    if is_zero(a) {
        (zero, 0)
    } else if exponent(a) == 0xff {
        (128, a & FRACTION)
    } else {
        normalized(a)
    }
}

/// `recip0.s`: an estimate of `1 / a`. A NaN keeps its sign and takes the estimate as its top
/// fraction bits.
pub(crate) fn recip0(a: u32) -> Out {
    let sign = a & SIGN;
    if is_nan(a) {
        let estimate = u32::from(reciprocal(a & FRACTION));
        let flags = if is_signaling(a) { INVALID } else { 0 };
        return (sign | INF | estimate << 15, flags);
    }
    if is_zero(a) {
        return (sign | INF, DIVIDE_BY_ZERO);
    }
    if is_inf(a) {
        return (sign, 0);
    }
    let (e, fraction) = normalized(a);
    let estimate = reciprocal(fraction);
    (seed(negative(a), estimate, -e - 8), 0)
}

/// `div0.s`: the reciprocal estimate of the fraction, halved for an odd exponent. No flag.
pub(crate) fn div0(a: u32) -> u32 {
    let (e, fraction) = seed_input(a, 0);
    let estimate = reciprocal(fraction);
    seed(negative(a), estimate, -8 - (e & 1))
}

/// `sqrt0.s`: the reciprocal square-root estimate of the fraction. No flag.
pub(crate) fn sqrt0(a: u32) -> u32 {
    let (e, fraction) = seed_input(a, -127);
    let estimate = rsqrt(e, fraction);
    seed(negative(a), estimate, -8)
}

/// `rsqrt0.s`: an estimate of `1 / sqrt(a)`. A NaN, a negative value or -inf gives a quiet NaN
/// with the estimate in fraction bits 16 to 22, so the estimate's top bit is lost.
pub(crate) fn rsqrt0(a: u32) -> Out {
    let sign = a & SIGN;
    let nan = |e: i32, fraction: u32| {
        let estimate = u32::from(rsqrt(e, fraction));
        sign | DEFAULT_NAN | ((estimate << 16) & FRACTION)
    };
    if is_nan(a) {
        let flags = if is_signaling(a) { INVALID } else { 0 };
        return (nan(128, a & FRACTION), flags);
    }
    if is_zero(a) {
        return (sign | INF, DIVIDE_BY_ZERO);
    }
    if is_inf(a) {
        if negative(a) {
            return (nan(128, 0), INVALID);
        }
        return (0, 0);
    }
    let (e, fraction) = normalized(a);
    if negative(a) {
        return (nan(e, fraction), INVALID);
    }
    let estimate = rsqrt(e, fraction);
    (seed(false, estimate, -8 - (e >> 1)), 0)
}
