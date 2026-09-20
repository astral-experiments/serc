use crate::common;

#[test]
fn matches_cpython_integer_floor_division_and_modulo() {
    for (left, right) in [
        ("7", "3"),
        ("6", "3"),
        ("2", "3"),
        ("0", "3"),
        ("9223372036854775808", "3"),
        ("18446744073709551616", "3"),
        (
            "340282366920938463463374607431768211457",
            "18446744073709551617",
        ),
        (
            "340282366920938463463374607431768211456",
            "18446744073709551616",
        ),
        ("3", "340282366920938463463374607431768211457"),
    ] {
        for left_sign in ["", "-"] {
            for right_sign in ["", "-"] {
                for operator in ["//", "%"] {
                    let source = format!(
                        "def f():\n    return {left_sign}{left} {operator} {right_sign}{right}\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "integer_floor_modulo.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_boolean_integer_division_and_zero_errors() {
    for (left, right) in [
        ("True", "2"),
        ("True", "-2"),
        ("False", "-2"),
        ("2", "True"),
        ("-2", "True"),
        ("True", "True"),
        ("False", "True"),
        ("18446744073709551616", "True"),
        ("7", "0"),
        ("-7", "0"),
        ("True", "False"),
        ("False", "False"),
        ("18446744073709551616", "0"),
    ] {
        for operator in ["//", "%"] {
            let source = format!("def f():\n    return {left} {operator} {right}\n");
            common::assert_matches_cpython_marshal(&source, "integer_division_limits.py");
        }
    }
}
