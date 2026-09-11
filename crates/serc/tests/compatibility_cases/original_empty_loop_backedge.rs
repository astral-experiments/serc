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

#[test]
fn copied_match_condition_backedges_keep_their_original_order() {
    for (filename, source) in [
        (
            "match_and_assignment_backedges.py",
            r"def f(values, left, right):
    result = None
    for value in values:
        match value:
            case 1:
                if left and right:
                    result = value
                    left = False
                    right = True
    return result
",
        ),
        (
            "match_and_nested_return_backedges.py",
            r"def f(values, left, right, accept):
    for value in values:
        match value:
            case 0:
                return False
            case 1:
                if left and right:
                    if accept(value):
                        return True
    return False
",
        ),
        (
            "nested_match_or_and_yield_backedges.py",
            r"def f(groups, left, right):
    for group in groups:
        for value in group:
            match value:
                case [*items] | (*items,):
                    if any(left(item) for item in items) and not any(
                        right(item) for item in items
                    ):
                        yield value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn shared_original_for_return_backedges_keep_threading() {
    common::assert_matches_cpython_marshal(
        r"def f(items, attempts, first, second):
    for attempt in attempts:
        for item in items:
            work(item)
            if first():
                break
        else:
            prepare()
            if first() and second():
                return attempt
    return None
",
        "compound_conditional_return.py",
    );
}
