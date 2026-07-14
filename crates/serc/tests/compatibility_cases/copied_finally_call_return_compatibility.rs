use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_call_returns() {
    let cases = [
        (
            "call_positive.py",
            r"
def call_positive(flag):
    try:
        return produce()
    finally:
        if flag:
            consume()
",
        ),
        (
            "call_control_following.py",
            r"
def call_control_following(flag):
    try:
        result = produce()
    finally:
        if flag:
            consume()
    return result
",
        ),
        (
            "name_return_control.py",
            r"
def name_return_control(flag):
    result = produce()
    try:
        return result
    finally:
        if flag:
            consume()
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
