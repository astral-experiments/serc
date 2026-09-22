use crate::common;

#[test]
fn matches_cpython_boolean_sequence_repetition() {
    for sequence in [
        "'text'",
        "''",
        "b'bytes'",
        "b''",
        "()",
        "(1,)",
        "('text', 1)",
        r"'\ud800'",
    ] {
        for boolean in ["False", "True"] {
            for expression in [
                format!("{sequence} * {boolean}"),
                format!("{boolean} * {sequence}"),
            ] {
                for source in [
                    format!("value = {expression}\n"),
                    format!("def f():\n    return {expression}\n"),
                ] {
                    common::assert_matches_cpython_marshal(
                        &source,
                        "boolean_sequence_repetition.py",
                    );
                }
            }
        }
    }
}
