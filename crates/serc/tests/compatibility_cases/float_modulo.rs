use crate::common;

#[test]
fn matches_cpython_float_modulo_signs_and_rounding() {
    for (left, right) in [
        ("1.5", "1.0"),
        ("1.0", "0.1"),
        ("7", "3.0"),
        ("7.0", "3"),
        ("9007199254740993", "3.0"),
        ("18446744073709551617", "3.0"),
        ("3.0", "18446744073709551617"),
        ("0", "3.0"),
        ("0.0", "3"),
        ("4.0", "2.0"),
        ("5e-324", "2.0"),
        ("1e308", "1e-308"),
    ] {
        for left_sign in ["", "-"] {
            for right_sign in ["", "-"] {
                common::assert_matches_cpython_marshal(
                    &format!(
                        "def remainder():\n    return {left_sign}{left} % {right_sign}{right}\n"
                    ),
                    "float_modulo.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_float_modulo_exceptional_values() {
    for expression in [
        "True % 0.1",
        "False % -3.0",
        "-1.5 % True",
        "1.5 % False",
        "1.0 % 0",
        "1 % -0.0",
        "1e309 % 1",
        "-1e309 % 1",
        "1 % 1e309",
        "-1 % 1e309",
        "1 % -1e309",
        "1e309 % 1e309",
        "(1e309 - 1e309) % 1",
        "1 % (1e309 - 1e309)",
        "1.0 % 1j",
    ] {
        common::assert_matches_cpython_marshal(
            &format!("def remainder():\n    return {expression}\n"),
            "float_modulo_limits.py",
        );
    }
    let huge = format!("0x1{}", "0".repeat(300));
    for expression in [format!("{huge} % 1.0"), format!("1.0 % {huge}")] {
        common::assert_matches_cpython_marshal(
            &format!("def remainder():\n    return {expression}\n"),
            "float_modulo_overflow.py",
        );
    }
}
