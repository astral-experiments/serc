use crate::common;

#[test]
fn async_call_handler() {
    common::assert_matches_cpython_marshal(
        r"async def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    await consume(item)
        except Exception:
            recover(item)
",
        "async_call_handler.py",
    );
}

#[test]
fn control_assignment_raise_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception:
            failed = item
            raise
",
        "control_assignment_raise_handler.py",
    );
}

#[test]
fn control_continue_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception:
            continue
",
        "control_continue_handler.py",
    );
}

#[test]
fn control_no_try() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume):
    for item in items:
        if outer(item):
            if inner(item):
                consume(item)
",
        "control_no_try.py",
    );
}

#[test]
fn control_pass_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception:
            pass
",
        "control_pass_handler.py",
    );
}

#[test]
fn control_raise_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception:
            raise
",
        "control_raise_handler.py",
    );
}

#[test]
fn control_single_if() {
    common::assert_matches_cpython_marshal(
        r"def f(items, inner, consume, recover):
    for item in items:
        try:
            if inner(item):
                consume(item)
        except Exception:
            recover(item)
",
        "control_single_if.py",
    );
}

#[test]
fn named_linear_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception as exc:
            recover(exc)
            failures.append(item)
",
        "named_linear_handler.py",
    );
}

#[test]
fn sync_assignment_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception:
            failed = item
",
        "sync_assignment_handler.py",
    );
}

#[test]
fn sync_call_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    consume(item)
        except Exception:
            recover(item)
",
        "sync_call_handler.py",
    );
}

#[test]
fn with_call_handler() {
    common::assert_matches_cpython_marshal(
        r"def f(items, outer, inner, consume, recover, manager):
    with manager:
        for item in items:
            try:
                if outer(item):
                    if inner(item):
                        consume(item)
            except Exception:
                recover(item)
",
        "with_call_handler.py",
    );
}
