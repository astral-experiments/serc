use crate::common;

#[test]
fn matches_cpython_match_or_implicit_return_boundaries() {
    for (scope, prefix) in [("async ", ""), ("", "    yield subject\n")] {
        for pattern in ["0 | 1", "0 | 1 | 2"] {
            for body in ["pass", "pass\n            pass"] {
                let source = format!(
                    "{scope}def classify(subject):\n{prefix}    match subject:\n        case {pattern}:\n            {body}\n        case _:\n            pass\n"
                );
                common::assert_matches_cpython_marshal(&source, "match_or_implicit_return.py");
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_multiline_match_or_locations() {
    let patterns = [
        "1 | 2 | 3",
        "(1\n            | 2\n            | 3)",
        "(\n            1\n            | 2\n            | 3\n        )",
        "(One()\n            | Two()\n            | Three())",
        "(One(value)\n            | Two(value)\n            | Three(value))",
        "([1, value]\n            | [2, value]\n            | [3, value])",
    ];
    let bodies = [
        "return None",
        "return answer",
        "return 42",
        "answer = 42",
        "pass",
        "consume(answer)",
    ];
    for pattern in patterns {
        for body in bodies {
            let source = format!(
                "def classify(subject, answer):\n    match subject:\n        case {pattern}:\n            {body}\n        case _:\n            return False\n"
            );
            common::assert_matches_cpython_marshal(&source, "multiline_match_or_locations.py");
        }
    }
}
