use crate::common;

#[test]
fn matches_cpython_float_remainder_folding() {
    for left in ["0.0", "0", "1.0", "1", "389.772", "21", "49.3", "1e309"] {
        for right in ["0.0", "0", "0.1", "3", "360", "1e309"] {
            if !left.contains(['.', 'e']) && !right.contains(['.', 'e']) {
                continue;
            }
            for (left_sign, right_sign) in [("", ""), ("-", ""), ("", "-"), ("-", "-")] {
                let source = format!(
                    "def remainder():\n    return {left_sign}{left} % {right_sign}{right}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    &format!("remainder_{left_sign}{left}_{right_sign}{right}.py"),
                );
            }
        }
    }
}

#[test]
fn matches_cpython_float_remainder_integer_conversion() {
    for expression in [
        "True % 0.1",
        "1.0 % False",
        "1000000000000000000000 % 0.1",
        "1.5 % 1000000000000000000000",
        "9007199254740993 % 2.0",
        "9007199254740995 % 2.0",
        "0x10000000000000000000000000000000000000000000000000000000000000000000000000000000000 % 0.5",
    ] {
        let source = format!("def remainder():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "float_remainder_conversion.py");
    }
}

#[test]
fn matches_cpython_mixed_big_integer_float_folding() {
    for integer in [
        "18446744073709551616",
        "18446744073709553665",
        "1000000000000000000000",
    ] {
        for operator in ["+", "-", "*", "/", "**"] {
            for sign in ["", "-"] {
                let float = if operator == "**" { "2.0" } else { "1.5" };
                for (left, right) in [
                    (format!("{sign}{integer}"), float.to_owned()),
                    (float.to_owned(), format!("{sign}{integer}")),
                ] {
                    let source =
                        format!("def mixed():\n    return ({left}) {operator} ({right})\n");
                    common::assert_matches_cpython_marshal(&source, "mixed_big_integer_float.py");
                }
            }
        }
    }
    let overflow = format!("0x1{}", "0".repeat(256));
    common::assert_matches_cpython_marshal(
        &format!("values = ({overflow} * 0.5, 0.5 / {overflow}, {overflow} ** 0.0)\n"),
        "mixed_big_integer_float_overflow.py",
    );
}
