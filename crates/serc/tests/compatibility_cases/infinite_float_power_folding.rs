use crate::common;

#[test]
fn matches_cpython_infinite_float_power_folding() {
    for base in ["1e309", "-1e309", "2.0", "-2.0", "0.5", "-0.0", "0", "1"] {
        for exponent in ["2", "3.0", "-3.0", "0.5", "-0.5", "1e309", "-1e309"] {
            let source = format!("def power():\n    return ({base}) ** ({exponent})\n");
            common::assert_matches_cpython_marshal(&source, "infinite_float_power.py");
        }
    }
}

#[test]
fn matches_cpython_float_power_overflow_and_nan_controls() {
    for expression in [
        "1e308 ** 2.0",
        "(-1e308) ** 3.0",
        "1e-308 ** -2.0",
        "0.0 ** -2.0",
        "(-0.0) ** -3.0",
        "(1e309-1e309) ** 0.0",
        "(1e309-1e309) ** 2.0",
        "1.0 ** (1e309-1e309)",
        "2.0 ** (1e309-1e309)",
    ] {
        let source = format!("def power():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "float_power_controls.py");
    }
}

#[test]
fn matches_cpython_float_power_nan_operand_signs() {
    let nans = ["1e309 - 1e309", "-(1e309 - 1e309)"];
    let reals = ["0", "-0.0", "1", "1.0", "-1.0", "2.0", "1e309"];
    for nan in nans {
        for real in reals {
            for (base, exponent) in [(nan, real), (real, nan)] {
                let source = format!("def power():\n    return ({base}) ** ({exponent})\n");
                common::assert_matches_cpython_marshal(&source, "float_power_nan_sign.py");
            }
        }
        for other_nan in nans {
            let source = format!("def power():\n    return ({nan}) ** ({other_nan})\n");
            common::assert_matches_cpython_marshal(&source, "float_power_nan_sign.py");
        }
    }
}

#[test]
fn executes_float_power_with_nan_operand_signs() {
    common::assert_executes(
        r"
from math import copysign

for value in (
    (1e309 - 1e309) ** 1,
    (-(1e309 - 1e309)) ** 1,
    (1e309 - 1e309) ** (-(1e309 - 1e309)),
    (-(1e309 - 1e309)) ** (1e309 - 1e309),
    0 ** (1e309 - 1e309),
    (1e309 - 1e309) ** 0,
    1 ** (1e309 - 1e309),
):
    print(copysign(1, value))
",
        "-1.0\n1.0\n-1.0\n1.0\n-1.0\n1.0\n1.0\n",
    );
}

#[test]
fn executes_infinite_float_powers_and_preserves_errors() {
    common::assert_executes(
        r"
print(1e309 ** 2, 2.0 ** 1e309, (-1e309) ** 3, (-1e309) ** -3)
try:
    value = 1e308 ** 2.0
except OverflowError:
    print('overflow')
try:
    value = 0.0 ** -2.0
except ZeroDivisionError:
    print('zero division')
",
        "inf inf -inf -0.0\noverflow\nzero division\n",
    );
}
