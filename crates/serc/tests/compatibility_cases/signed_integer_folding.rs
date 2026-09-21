use crate::common;

#[test]
fn matches_cpython_signed_integer_power_folding() {
    for base in ["0", "1", "2", "3", "18446744073709551616"] {
        for exponent in [
            "0",
            "1",
            "2",
            "3",
            "60",
            "61",
            "64",
            "65",
            "129",
            "999999999999999999999999999999999999999",
        ] {
            for sign in ["", "-"] {
                let source = format!("def power():\n    return ({sign}{base}) ** {exponent}\n");
                common::assert_matches_cpython_marshal(&source, "signed_integer_power.py");
            }
        }
    }
    common::assert_matches_cpython_marshal(
        "values = (True ** True, False ** False, (-3) ** True, True ** 128, True ** 129)\n",
        "boolean_integer_power.py",
    );
}

#[test]
fn matches_cpython_signed_integer_divmod_folding() {
    for (left, right) in [
        ("0", "3"),
        ("2", "3"),
        ("12", "3"),
        ("13", "10"),
        ("18446744073709551615", "18446744073709551616"),
        ("18446744073709551616", "18446744073709551615"),
        (
            "123456789012345678901234567890123456789012345678901234567890",
            "12345678901234567890123",
        ),
        ("1", "0"),
        ("0", "0"),
    ] {
        for operator in ["//", "%"] {
            for (left_sign, right_sign) in [("", ""), ("-", ""), ("", "-"), ("-", "-")] {
                let source = format!(
                    "def divide():\n    return {left_sign}{left} {operator} {right_sign}{right}\n"
                );
                common::assert_matches_cpython_marshal(&source, "signed_integer_divmod.py");
            }
        }
    }
    common::assert_matches_cpython_marshal(
        "values = (True // True, True % True, False // True, -3 // True, True % -3, 1 // False)\n",
        "boolean_integer_divmod.py",
    );
}
