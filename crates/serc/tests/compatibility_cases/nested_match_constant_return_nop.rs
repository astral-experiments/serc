use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_nested_match_call_exit() {
    assert_matches_cpython_marshal(
        r"def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    work(item)
                case _:
                    work(item)
    return None
",
        "nested_match_call_exit.py",
    );
}

#[test]
fn matches_cpython_nested_match_call_exit_live_continuation_control() {
    assert_matches_cpython_marshal(
        r"def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    work(item)
                case _:
                    work(item)
            work(value)
    return None
",
        "nested_match_live_continuation_control.py",
    );
}
