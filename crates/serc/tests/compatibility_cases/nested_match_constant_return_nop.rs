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

#[test]
fn matches_cpython_nested_match_shared_conditional_return() {
    assert_matches_cpython_marshal(
        r"def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    if work(item):
                        work(value)
                case None:
                    pass
                case _:
                    work(item)
        case _:
            raise ValueError(value)
    return True
",
        "nested_match_conditional_fallthrough_control.py",
    );
}

#[test]
fn matches_cpython_nested_match_local_return_with_comprehension() {
    assert_matches_cpython_marshal(
        r"def f(value, work, values):
    events = [work(element) for element in values]
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    if work(item):
                        work(value)
                case _:
                    work(item)
        case _:
            raise ValueError(value)
    return events
",
        "nested_match_local_return_comprehension.py",
    );
}

#[test]
fn matches_cpython_nested_match_local_return_without_comprehension() {
    assert_matches_cpython_marshal(
        r"def f(value, work, values):
    events = values
    match value:
        case Outer(item=item):
            match item:
                case Inner():
                    if work(item):
                        work(value)
                case _:
                    work(item)
        case _:
            raise ValueError(value)
    return events
",
        "nested_match_local_return_plain_control.py",
    );
}
