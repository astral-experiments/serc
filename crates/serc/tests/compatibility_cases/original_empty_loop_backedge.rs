use crate::common;

#[test]
fn nested_for_else_pass_backedge_location() {
    common::assert_matches_cpython_marshal(
        r"def f(items, attempts, ready):
    for attempt in attempts:
        for item in items:
            work(item)
            if ready():
                break
        else:
            prepare()
            if not ready():
                return attempt
            else:
                pass
    return None
",
        "nested_for_else_pass_backedge_location.py",
    );
}

#[test]
fn preserves_nested_for_else_backedge_donors() {
    for (name, tail) in [
        ("conditional_pass", "if ready():\n    pass"),
        ("conditional_return", "if not ready():\n    return attempt"),
        (
            "located_else",
            "if not ready():\n    return attempt\nelse:\n    finish()",
        ),
    ] {
        let source = format!(
            "def f(items, attempts, ready):\n    for attempt in attempts:\n        for item in items:\n            work(item)\n            if ready():\n                break\n        else:\n            prepare()\n            {}\n    return None\n",
            tail.replace('\n', "\n            "),
        );
        common::assert_matches_cpython_marshal(
            &source,
            &format!("nested_for_else_backedge_{name}.py"),
        );
    }
}

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
