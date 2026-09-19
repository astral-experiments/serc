use crate::common;

fn shifted_hex(value: u64, shift: usize) -> String {
    format!(
        "0x{:x}{}",
        u128::from(value) << (shift % 4),
        "0".repeat(shift / 4)
    )
}

#[test]
fn matches_cpython_signed_bitwise_limb_boundaries() {
    for bit in [15_usize, 30, 64, 128, 4096] {
        let left = shifted_hex(1, bit);
        let right = shifted_hex(3, bit - 1);
        for (left_sign, right_sign) in [("", ""), ("-", ""), ("", "-"), ("-", "-")] {
            for operator in ["&", "|", "^"] {
                let source = format!(
                    "def f():\n    return {left_sign}{left} {operator} {right_sign}{right}\n"
                );
                common::assert_matches_cpython_marshal(&source, "signed_bitwise_limbs.py");
            }
        }
        for sign in ["", "-"] {
            common::assert_matches_cpython_marshal(
                &format!("value = ~({sign}{left})\n"),
                "signed_bitwise_invert.py",
            );
        }
    }
}

#[test]
fn matches_cpython_bitwise_boolean_types_and_zero() {
    for left in ["False", "True"] {
        for right in ["False", "True", "0", "1", "-1"] {
            for operator in ["&", "|", "^"] {
                let source = format!(
                    "values = ({left} {operator} {right}, {right} {operator} {left}, False, True, 0, 1)\n"
                );
                common::assert_matches_cpython_marshal(&source, "bitwise_boolean_types.py");
            }
        }
    }
    common::assert_matches_cpython_marshal(
        "values = (~0, ~(-1), ~(~0), ~False, ~True, -0 & -1, -1 ^ -1, -1 & 0, -1 | 0)\n",
        "bitwise_normalized_zero.py",
    );
}

#[test]
fn matches_cpython_nested_signed_bitwise_folds() {
    common::assert_matches_cpython_marshal(
        "def f(value):\n    initial = (-256 << 32) | 1\n    value &= ~(0xF << 76)\n    value |= 0x4 << 76\n    value &= ~(0b11 << 62)\n    return value, initial\n",
        "bitwise_quantizer_and_uuid_masks.py",
    );
    for expression in [
        "(~(1 << 80) & ((1 << 96) - 1)) ^ -1",
        "((1 << 80) ^ -(1 << 80)) + 3",
        "(~(-1) | False) & -7",
        "(~(1 << 80)) / ((1 << 80) | 1)",
    ] {
        for source in [
            format!("def f(value={expression}):\n    return value\n"),
            format!("def f():\n    if {expression}:\n        return 1\n    return 0\n"),
        ] {
            common::assert_matches_cpython_marshal(&source, "nested_bitwise_folds.py");
        }
    }
}

#[test]
fn matches_cpython_bitwise_integer_complex_conversion() {
    for bit in [80, 1024, 4096] {
        let integer = shifted_hex(1, bit);
        for operator in ["+", "*", "/"] {
            common::assert_matches_cpython_marshal(
                &format!("def f():\n    return (~{integer}) {operator} 1j\n"),
                "bitwise_complex_conversion.py",
            );
        }
    }
    for expression in [
        format!("(~{}) * 0j", shifted_hex(1, 1024)),
        format!("(~{}) / 0j", shifted_hex(1, 80)),
        "(True & False) + 1j".to_string(),
        "(~(-1)) * -0j".to_string(),
        "(True | False) - 0j".to_string(),
        format!("(~{}) * 0j", shifted_hex(1, 80)),
        format!("0j * (~{})", shifted_hex(1, 1023)),
        "0j + -0.0".to_string(),
        "-0.0 + -0j".to_string(),
        "-0j - -0.0".to_string(),
        "-0.0 - 0j".to_string(),
        "-1 * 0j".to_string(),
        "0j * -1.0".to_string(),
    ] {
        common::assert_matches_cpython_marshal(
            &format!("def f():\n    return {expression}\n"),
            "bitwise_complex_failure_or_zero.py",
        );
    }
}
