use crate::common;

#[test]
fn match_or_pass() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        match item:
            case True | False:
                pass
            case _:
                raise ValueError(item)
",
        "match_or_pass.py",
    );
}

#[test]
fn match_literal_pass() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        match item:
            case 1:
                pass
            case _:
                raise ValueError(item)
",
        "match_literal_pass.py",
    );
}

#[test]
fn for_simple_pass() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        pass
",
        "for_simple_pass.py",
    );
}

#[test]
fn match_or_work() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        match item:
            case True | False:
                work(item)
            case _:
                raise ValueError(item)
",
        "match_or_work.py",
    );
}

#[test]
fn for_body_call() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        work(item)
",
        "for_body_call.py",
    );
}

#[test]
fn for_runtime_if() {
    common::assert_matches_cpython_marshal(
        r"def f(items):
    for item in items:
        if ready(item):
            work(item)
        else:
            finish(item)
",
        "for_runtime_if.py",
    );
}
