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
