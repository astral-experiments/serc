use crate::common;

#[test]
fn matches_cpython_general_complex_power_folding() {
    for base in ["1j", "2j", "1+2j", "-2+1j", "-1-0j"] {
        for exponent in ["0.5", "-0.5", "101", "-101", "1j", "2-3j"] {
            let source = format!("def power():\n    return ({base}) ** ({exponent})\n");
            common::assert_matches_cpython_marshal(&source, "complex_power.py");
        }
    }
}

#[test]
fn matches_cpython_complex_power_limits() {
    for expression in [
        "0j ** 0.5",
        "0j ** 101",
        "0j ** -0.5",
        "0j ** 1j",
        "1j ** 100",
        "1j ** -100",
        "(1e308+1e308j) ** 1.5",
        "(1e-308+1e-308j) ** -1.5",
        "(1e-308+1e-308j) ** 1.5",
        "(1e308+1e308j) ** -1.5",
        "(5e-324j) ** 0.5",
        "2j ** (1e309)",
        "2j ** (-1e309)",
        "1j ** (1e309j)",
        "(1e309j) ** 0.5",
        "(1e309j) ** -0.5",
    ] {
        let source = format!("def power():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "complex_power_limits.py");
    }
}

#[test]
fn matches_cpython_complex_power_infinite_phase() {
    // CPython follows the platform libm's errno behavior for sin/cos of infinity.
    // Cover both infinite exponents and overflow in a phase from finite inputs.
    for expression in [
        "1j ** (-1e309)",
        "2j ** (1e309j)",
        "(-1+1j) ** 1e308",
        "(-1+1j) ** -1e308",
    ] {
        let source = format!("def power():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "complex_power_infinite_phase.py");
    }
}

#[test]
fn matches_cpython_nonfinite_complex_power_folding() {
    for base in [
        "1e309j",
        "-1e309j",
        "(1e309+0j)",
        "(-1e309-0j)",
        "(1e309+1e309j)",
        "(1e309j-1e309j)",
    ] {
        for exponent in [
            "-1", "-1.0", "(-1+0j)", "(-1-0j)", "-2", "0", "1", "2", "1j",
        ] {
            let source = format!("def power():\n    return ({base}) ** ({exponent})\n");
            common::assert_matches_cpython_marshal(&source, "nonfinite_complex_power.py");
        }
    }
}

#[test]
fn executes_nonfinite_complex_powers_with_cpython_signs() {
    common::assert_executes(
        r"
import math

def signs(value):
    return math.copysign(1, value.real), math.copysign(1, value.imag)

def dynamic(base, exponent):
    return base ** exponent

for base, exponent, folded in [
    (1e309j, -1, 1e309j ** -1),
    (-1e309j, -1.0, (-1e309j) ** -1.0),
    (1e309j, -1+0j, 1e309j ** (-1+0j)),
    (1e309j-1e309j, 1j, (1e309j-1e309j) ** 1j),
]:
    print(signs(folded) == signs(dynamic(base, exponent)))
",
        "True\nTrue\nTrue\nTrue\n",
    );
}

#[test]
fn executes_complex_powers_and_preserves_errors() {
    common::assert_executes(
        r"
import struct

def bits(value):
    return struct.pack('dd', value.real, value.imag)

pairs = [(1+2j, 0.5), (1j, 101), (2j, 1j)]
folded = [(1+2j)**0.5, 1j**101, 2j**1j]
print(all(bits(a**b) == bits(value) for (a, b), value in zip(pairs, folded)))
try:
    value = 0j ** -0.5
except ZeroDivisionError:
    print('zero division')
try:
    value = (1e308+1e308j) ** 1.5
except OverflowError:
    print('overflow')
",
        "True\nzero division\noverflow\n",
    );
}
