//! Arithmetic used by the two identified CPython 3.14.5 Linux builds.
//!
//! These operations model the floating-point instruction order of the specific
//! reference binaries, including NaN payload propagation. They do not claim to
//! describe other CPython builds that happen to have the same version or magic.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Linux314Profile {
    ActionsGcc,
    UvClang,
}

impl Linux314Profile {
    pub(super) fn from_config(profile: crate::NativeBuildProfile) -> Option<Self> {
        match profile {
            crate::NativeBuildProfile::Default => None,
            crate::NativeBuildProfile::Cpython3145ActionsGcc => Some(Self::ActionsGcc),
            crate::NativeBuildProfile::Cpython3145UvClang => Some(Self::UvClang),
        }
    }
}

const EXP: u64 = 0x7ff0_0000_0000_0000;
const FRACTION: u64 = 0x000f_ffff_ffff_ffff;
const QUIET: u64 = 0x0008_0000_0000_0000;
const SIGN: u64 = 0x8000_0000_0000_0000;

fn is_nan(bits: u64) -> bool {
    bits & EXP == EXP && bits & FRACTION != 0
}

fn is_infinite(bits: u64) -> bool {
    bits & !SIGN == EXP
}

fn is_zero(bits: u64) -> bool {
    bits & !SIGN == 0
}

/// Model a scalar x86 operation with the first value in its destination.
pub(super) fn binary(first: f64, second: f64, operator: char) -> f64 {
    let (a, b) = (first.to_bits(), second.to_bits());
    if is_nan(a) {
        return f64::from_bits(a | QUIET);
    }
    if is_nan(b) {
        return f64::from_bits(b | QUIET);
    }
    let invalid = match operator {
        '+' => is_infinite(a) && is_infinite(b) && (a ^ b) & SIGN != 0,
        '-' => is_infinite(a) && is_infinite(b) && (a ^ b) & SIGN == 0,
        '*' => is_infinite(a) && is_zero(b) || is_zero(a) && is_infinite(b),
        '/' => is_zero(a) && is_zero(b) || is_infinite(a) && is_infinite(b),
        _ => unreachable!(),
    };
    if invalid {
        return f64::from_bits(SIGN | EXP | QUIET);
    }
    match operator {
        '+' => first + second,
        '-' => first - second,
        '*' => first * second,
        '/' => first / second,
        _ => unreachable!(),
    }
}

fn neg(value: f64) -> f64 {
    f64::from_bits(value.to_bits() ^ SIGN)
}

/// Fold the actual complex-left, real-right addition dispatch of each build.
pub(super) fn complex_plus_real(complex: f64, real: f64, profile: Linux314Profile) -> f64 {
    match profile {
        Linux314Profile::ActionsGcc => binary(complex, real, '+'),
        Linux314Profile::UvClang => binary(real, complex, '+'),
    }
}

/// Fold `_Py_c_prod`, including CPython's recovery for non-finite operands.
pub(super) fn product(
    (mut a, mut b): (f64, f64),
    (mut c, mut d): (f64, f64),
    profile: Linux314Profile,
) -> (f64, f64) {
    let add = |x, y| binary(x, y, '+');
    let sub = |x, y| binary(x, y, '-');
    let mul = |x, y| binary(x, y, '*');
    let gcc = profile == Linux314Profile::ActionsGcc;
    let (ac, bd, ad, bc) = if gcc {
        (mul(a, c), mul(b, d), mul(a, d), mul(b, c))
    } else {
        (mul(c, a), mul(d, b), mul(d, a), mul(c, b))
    };
    let (mut real, mut imag) = (sub(ac, bd), if gcc { add(ad, bc) } else { add(bc, ad) });
    if real.is_nan() && imag.is_nan() {
        let boxed = |x: f64| if x.is_infinite() { 1.0_f64 } else { 0.0 }.copysign(x);
        let zero_nan = |x: f64| if x.is_nan() { 0.0_f64.copysign(x) } else { x };
        let mut recalculate = false;
        if a.is_infinite() || b.is_infinite() {
            (a, b, c, d) = (boxed(a), boxed(b), zero_nan(c), zero_nan(d));
            recalculate = true;
        }
        if c.is_infinite() || d.is_infinite() {
            (a, b, c, d) = (zero_nan(a), zero_nan(b), boxed(c), boxed(d));
            recalculate = true;
        }
        if !recalculate && [ac, bd, ad, bc].into_iter().any(f64::is_infinite) {
            (a, b, c, d) = (zero_nan(a), zero_nan(b), zero_nan(c), zero_nan(d));
            recalculate = true;
        }
        if recalculate {
            real = mul(f64::INFINITY, sub(mul(a, c), mul(b, d)));
            imag = mul(f64::INFINITY, add(mul(a, d), mul(b, c)));
        }
    }
    (real, imag)
}

/// Fold `_Py_c_quot`, including CPython's recovery for non-finite operands.
pub(super) fn quotient(
    (a, b): (f64, f64),
    (c, d): (f64, f64),
    profile: Linux314Profile,
) -> Option<(f64, f64)> {
    let add = |x, y| binary(x, y, '+');
    let sub = |x, y| binary(x, y, '-');
    let mul = |x, y| binary(x, y, '*');
    let div = |x, y| binary(x, y, '/');
    let gcc = profile == Linux314Profile::ActionsGcc;
    let (mut real, mut imag) = if c.abs() >= d.abs() {
        if c == 0.0 {
            return None;
        }
        let ratio = div(d, c);
        let denominator = add(if gcc { mul(ratio, d) } else { mul(d, ratio) }, c);
        let (real, imag) = if gcc {
            (add(mul(ratio, b), a), sub(b, mul(ratio, a)))
        } else {
            (add(a, mul(ratio, b)), add(b, mul(ratio, neg(a))))
        };
        (div(real, denominator), div(imag, denominator))
    } else if d.abs() >= c.abs() {
        let ratio = div(c, d);
        let denominator = add(if gcc { mul(ratio, c) } else { mul(c, ratio) }, d);
        let (real, imag) = if gcc {
            (add(mul(ratio, a), b), sub(mul(ratio, b), a))
        } else {
            (add(b, mul(ratio, a)), add(neg(a), mul(ratio, b)))
        };
        (div(real, denominator), div(imag, denominator))
    } else {
        (f64::from_bits(EXP | QUIET), f64::from_bits(EXP | QUIET))
    };
    if real.is_nan() && imag.is_nan() {
        if (a.is_infinite() || b.is_infinite()) && c.is_finite() && d.is_finite() {
            let boxed_real = if a.is_infinite() { 1.0_f64 } else { 0.0 }.copysign(a);
            let boxed_imag = if b.is_infinite() { 1.0_f64 } else { 0.0 }.copysign(b);
            real = mul(f64::INFINITY, add(mul(boxed_real, c), mul(boxed_imag, d)));
            imag = mul(f64::INFINITY, sub(mul(boxed_imag, c), mul(boxed_real, d)));
        } else if (c.is_infinite() || d.is_infinite()) && a.is_finite() && b.is_finite() {
            let boxed_real = if c.is_infinite() { 1.0_f64 } else { 0.0 }.copysign(c);
            let boxed_imag = if d.is_infinite() { 1.0_f64 } else { 0.0 }.copysign(d);
            real = mul(0.0, add(mul(a, boxed_real), mul(b, boxed_imag)));
            imag = mul(0.0, sub(mul(b, boxed_real), mul(a, boxed_imag)));
        }
    }
    Some((real, imag))
}

/// Apply the 3.14 mixed-operation dispatch before any constant is pooled.
pub(super) fn arithmetic(
    (a, b): (f64, f64),
    left_complex: bool,
    (c, d): (f64, f64),
    right_complex: bool,
    operator: ruff_python_ast::Operator,
    profile: Linux314Profile,
) -> (f64, f64) {
    use ruff_python_ast::Operator;
    match (left_complex, right_complex, operator) {
        (true, true, Operator::Add) => (binary(c, a, '+'), binary(d, b, '+')),
        (true, true, Operator::Sub) => (binary(a, c, '-'), binary(b, d, '-')),
        (true, true, Operator::Mult) => product((a, b), (c, d), profile),
        (true, false, Operator::Add) => (complex_plus_real(a, c, profile), b),
        (true, false, Operator::Sub) => (binary(a, c, '-'), b),
        (true, false, Operator::Mult) => (binary(c, a, '*'), binary(c, b, '*')),
        (false, true, Operator::Add) => (binary(c, a, '+'), d),
        (false, true, Operator::Sub) => (binary(a, c, '-'), neg(d)),
        (false, true, Operator::Mult) => (binary(c, a, '*'), binary(d, a, '*')),
        _ => unreachable!(),
    }
}

/// Preserve the mixed division formulas that avoid artificial zero components.
pub(super) fn divide(
    (a, b): (f64, f64),
    left_complex: bool,
    (c, d): (f64, f64),
    right_complex: bool,
    profile: Linux314Profile,
) -> Option<(f64, f64)> {
    if left_complex && right_complex {
        return quotient((a, b), (c, d), profile);
    }
    if left_complex {
        return (c != 0.0).then(|| (binary(a, c, '/'), binary(b, c, '/')));
    }
    let (mut real, mut imag) = if c.abs() >= d.abs() {
        if c == 0.0 {
            return None;
        }
        let ratio = binary(d, c, '/');
        let denominator = binary(binary(d, ratio, '*'), c, '+');
        (
            binary(a, denominator, '/'),
            binary(binary(neg(a), ratio, '*'), denominator, '/'),
        )
    } else if d.abs() >= c.abs() {
        let ratio = binary(c, d, '/');
        let denominator = binary(binary(c, ratio, '*'), d, '+');
        (
            binary(binary(a, ratio, '*'), denominator, '/'),
            binary(neg(a), denominator, '/'),
        )
    } else {
        (f64::from_bits(EXP | QUIET), f64::from_bits(EXP | QUIET))
    };
    if real.is_nan() && imag.is_nan() && (c.is_infinite() || d.is_infinite()) && a.is_finite() {
        let boxed_real = f64::from(c.is_infinite()).copysign(c);
        let boxed_imag = f64::from(d.is_infinite()).copysign(d);
        real = binary(0.0, binary(a, boxed_real, '*'), '*');
        imag = binary(0.0, binary(neg(a), boxed_imag, '*'), '*');
    }
    Some((real, imag))
}

/// Match each binary's general complex power instruction order and libm calls.
pub(super) fn general_power(
    (a, b): (f64, f64),
    (c, d): (f64, f64),
    profile: Linux314Profile,
) -> Option<(f64, f64)> {
    if c == 0.0 && d == 0.0 {
        return Some((1.0, 0.0));
    }
    if a == 0.0 && b == 0.0 {
        return if d != 0.0 || c < 0.0 {
            None
        } else {
            Some((0.0, 0.0))
        };
    }
    let mul = |x, y| binary(x, y, '*');
    let absolute = a.hypot(b);
    let mut magnitude = absolute.powf(c);
    let angle = b.atan2(a);
    let mut phase = mul(c, angle);
    let gcc = profile == Linux314Profile::ActionsGcc;
    if d != 0.0 {
        let factor = mul(neg(angle), d).exp();
        magnitude = if gcc {
            mul(factor, magnitude)
        } else {
            mul(magnitude, factor)
        };
        phase = binary(phase, mul(absolute.ln(), d), '+');
    }
    if phase.is_infinite() {
        return None;
    }
    let (sine, cosine) = if gcc {
        phase.sin_cos()
    } else {
        (phase.sin(), phase.cos())
    };
    let real = mul(cosine, magnitude);
    let imag = if gcc {
        mul(magnitude, sine)
    } else {
        mul(sine, magnitude)
    };
    (!real.is_infinite() && !imag.is_infinite()).then_some((real, imag))
}

#[cfg(test)]
mod tests {
    use super::{Linux314Profile, complex_plus_real, product, quotient};

    #[test]
    fn identified_builds_preserve_their_complex_nan_bits() {
        let nan = f64::from_bits(0xfff8_0000_0000_0000);
        for denominator in [(1.0, 2.0), (2.0, 1.0)] {
            let gcc = quotient((nan, 1.0), denominator, Linux314Profile::ActionsGcc).unwrap();
            let uv = quotient((nan, 1.0), denominator, Linux314Profile::UvClang).unwrap();
            assert_eq!(
                (gcc.0.to_bits(), gcc.1.to_bits()),
                (0xfff8_0000_0000_0000, 0xfff8_0000_0000_0000)
            );
            assert_eq!(
                (uv.0.to_bits(), uv.1.to_bits()),
                (0xfff8_0000_0000_0000, 0x7ff8_0000_0000_0000)
            );
        }
        let positive = f64::from_bits(0x7ff8_0000_0000_0042);
        assert_eq!(
            complex_plus_real(positive, nan, Linux314Profile::ActionsGcc).to_bits(),
            positive.to_bits()
        );
        assert_eq!(
            complex_plus_real(positive, nan, Linux314Profile::UvClang).to_bits(),
            nan.to_bits()
        );
        for profile in [Linux314Profile::ActionsGcc, Linux314Profile::UvClang] {
            let value = product((2.0, 3.0), (4.0, 5.0), profile);
            assert_eq!(
                (value.0.to_bits(), value.1.to_bits()),
                ((-7.0_f64).to_bits(), 22.0_f64.to_bits())
            );
            assert!(quotient((1.0, 2.0), (0.0, -0.0), profile).is_none());
        }
    }
}
