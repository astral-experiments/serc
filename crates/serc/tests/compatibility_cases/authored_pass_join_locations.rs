use crate::common;

#[test]
fn matches_cpython_authored_pass_join_locations() {
    let source = include_str!("../fixtures/authored_pass_join_locations.py");
    common::assert_matches_cpython_marshal(source, "authored_pass_join_locations.py");
}

#[test]
fn matches_cpython_omitted_pass_at_normal_try_exit() {
    for (source, filename) in [
        (
            r"def f(item):
    try:
        work(item)
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "named_handler_footer.py",
        ),
        (
            r"def f(item):
    try:
        work(item)
        pass
    except Error:
        recover(item)
    observe(item)
    consume(item)
    finish(item)
",
            "unnamed_handler_footer.py",
        ),
        (
            r"def f(item):
    try:
        item = work(item)
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "assigned_call_footer.py",
        ),
        (
            r"def f(item):
    try:
        try:
            work(item)
            pass
        except Error as exc:
            recover(exc)
        observe(item)
    finally:
        finish(item)
    consume(item)
",
            "nested_finally_footer.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_omitted_pass_with_small_exit_and_existing_markers() {
    for (source, filename) in [
        (
            r"def f(item):
    try:
        work(item)
        pass
    except Error as exc:
        recover(exc)
    return None
",
            "short_return_footer.py",
        ),
        (
            r"def f(item):
    try:
        work(item); pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "same_line_pass_footer.py",
        ),
        (
            r"def f(item):
    try:
        work(item)
        42
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "constant_then_pass.py",
        ),
        (
            r"def f(item):
    try:
        pass
    except Error as exc:
        recover(exc)
    observe(item)
    consume(item)
    finish(item)
",
            "pass_only_footer.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
