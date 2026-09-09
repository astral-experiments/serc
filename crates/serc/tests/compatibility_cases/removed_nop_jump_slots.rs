use crate::common;

#[test]
fn matches_cpython_match_pass_return_copy_budget() {
    for scope in ["", "async "] {
        for pattern in [
            "Choice.first | None",
            "Choice.first | Choice.second",
            "0 | 1",
        ] {
            for wildcard in ["alternate()", "pass"] {
                let source = format!(
                    "{scope}def collect(value):\n    match value:\n        case {pattern}:\n            pass\n        case _:\n            {wildcard}\n    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "match_pass_return.py");
            }
        }
    }
}
