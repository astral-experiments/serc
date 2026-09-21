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
