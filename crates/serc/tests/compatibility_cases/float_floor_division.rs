use crate::common;

#[test]
fn matches_cpython_float_floor_division_signs_and_rounding() {
    for (left, right) in [
        ("1000000000000", "1e9"),
        ("1", "0.1"),
        ("1.0", "0.1"),
        ("7", "3.0"),
        ("7.0", "3"),
        ("9007199254740993", "1.0"),
        ("9007199254740995", "3.0"),
        ("0x800000000000000000000000000001", "3.0"),
        ("1.0", "0x800000000000000000000000000001"),
        ("0", "3.0"),
        ("0.0", "3"),
    ] {
        for left_sign in ["", "-"] {
            for right_sign in ["", "-"] {
                let source =
                    format!("def f():\n    return {left_sign}{left} // {right_sign}{right}\n");
                common::assert_matches_cpython_marshal(&source, "float_floor_division.py");
            }
        }
    }
}

#[test]
fn matches_cpython_float_floor_division_limits() {
    for expression in [
        "True // 0.1",
        "True // -0.1",
        "False // -3.0",
        "1.0 // True",
        "-1.0 // True",
        "1.0 // False",
        "1 // 0.0",
        "1.0 // 0",
        "1e309 // 0.0",
        "1e309 // 1",
        "1e309 // 1e309",
        "1 // 1e309",
        "-1 // 1e309",
        "1 // -1e309",
        "(1e309 - 1e309) // 1",
        "1 // (1e309 - 1e309)",
        "1e308 // 1e-308",
        "5e-324 // 2",
        "-5e-324 // 2",
    ] {
        let source = format!("def f():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "float_floor_division_limits.py");
    }
    let overflowing_integer = format!("1{}", "0".repeat(309));
    for (left, right) in [
        (overflowing_integer.as_str(), "1.0"),
        ("1.0", overflowing_integer.as_str()),
        (overflowing_integer.as_str(), "0.0"),
    ] {
        let source = format!("def f():\n    return {left} // {right}\n");
        common::assert_matches_cpython_marshal(&source, "float_floor_division_overflow.py");
    }
}
