use crate::common;

#[test]
fn matches_cpython_bigint_float_arithmetic_and_rounding() {
    for integer in [
        "18446744073709551616",
        "18446744073709553664",
        "18446744073709557760",
        "340282366920938463463374607431768211455",
    ] {
        for sign in ["", "-"] {
            for operator in ["+", "-", "*", "/"] {
                for (left, right) in [
                    (format!("{sign}{integer}"), "3.5".to_owned()),
                    ("3.5".to_owned(), format!("{sign}{integer}")),
                ] {
                    common::assert_matches_cpython_marshal(
                        &format!("def arithmetic():\n    return {left} {operator} {right}\n"),
                        "bigint_float_arithmetic.py",
                    );
                }
            }
        }
    }
    for expression in [
        "18446744073709551616 + 1.0",
        "18446744073709551616 ** 0.5",
        "(-18446744073709551616) ** -1.0",
        "1.0 ** 18446744073709551616",
        "18446744073709551616 / 0.0",
        "0.0 / -18446744073709551616",
        "18446744073709551616 * -0.0",
        "18446744073709551616 + 1e309",
        "1e309 - 18446744073709551616",
        "18446744073709551616 ** 100.0",
    ] {
        common::assert_matches_cpython_marshal(
            &format!("def arithmetic():\n    return {expression}\n"),
            "bigint_float_arithmetic_limits.py",
        );
    }
}

#[test]
fn preserves_bigint_float_conversion_overflow_at_runtime() {
    let huge = format!("0x1{}", "0".repeat(300));
    for operator in ["+", "-", "*", "/", "**"] {
        for (left, right) in [(huge.as_str(), "0.0"), ("0.0", huge.as_str())] {
            common::assert_matches_cpython_marshal(
                &format!("def arithmetic():\n    return {left} {operator} {right}\n"),
                "bigint_float_arithmetic_overflow.py",
            );
        }
    }
}
