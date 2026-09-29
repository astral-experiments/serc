use crate::common;

#[test]
fn matches_cpython_empty_bytes_from_constant_folding() {
    for expression in [
        "b'x' * 0",
        "0 * b'x'",
        "b'' * 3",
        "b'x'[:0]",
        "b'' + b''",
        "(b'x' * 0) * 2",
        "(b'x' * 0,)[0]",
    ] {
        for source in [
            format!("value = {expression}\n"),
            format!("def f():\n    return {expression}\n"),
        ] {
            common::assert_matches_cpython_marshal(&source, "empty_bytes_folding.py");
        }
    }
}
