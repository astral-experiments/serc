use crate::common;

#[test]
fn matches_cpython_negated_signed_integer_folds() {
    for number in ["0", "7", "9223372036854775807", "9223372036854775808"] {
        for expression in [
            format!("-(-{number})"),
            format!("-(0 - {number})"),
            format!("-(~(-{number}))"),
        ] {
            let source = format!("def f():\n    return {expression}\n");
            common::assert_matches_cpython_marshal(&source, "negated_signed_constant.py");
        }
    }
    common::assert_matches_cpython_marshal(
        "def f():\n    return -(-1 - 4 - 10) / 3\n",
        "negated_signed_arithmetic.py",
    );
}
