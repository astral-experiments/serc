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

#[test]
fn chained_comparison_pass_arms_keep_original_jump_boundaries() {
    for (filename, source) in [
        (
            "async_nested_chained_pass_assignment.py",
            r"async def run(fetch, low, high, output):
    items = await fetch()
    previous = None
    for value in items:
        if value is not None:
            if low <= value < high:
                pass
            else:
                previous = value
        output.append(previous)
    return output
",
        ),
        (
            "async_nested_chained_pass_call_assignment.py",
            r"async def run(fetch, low, high, reject, output):
    items = await fetch()
    previous = None
    for value in items:
        if value is not None:
            if low <= value < high:
                pass
            else:
                reject(value)
                previous = value
        output.append(previous)
    return output
",
        ),
        (
            "sync_boolean_chained_pass_elif_pass.py",
            r"def run(skip_empty, skip_busy, completed, failed, busy, report):
    if skip_empty and completed == failed == 0:
        pass
    elif skip_busy and not busy:
        pass
    else:
        report()
    return completed, failed
",
        ),
        (
            "chained_pass_small_return_control.py",
            r"def run(a, b, c, fallback):
    if a < b < c:
        pass
    else:
        fallback()
    return None
",
        ),
        (
            "negated_chained_pass_control.py",
            r"def run(a, b, c, fallback, output):
    if not a < b < c:
        pass
    else:
        fallback()
    output.append(a)
    return output
",
        ),
        (
            "same_line_chained_pass_control.py",
            r"def run(a, b, c, fallback, output):
    if a < b < c: pass
    else:
        fallback()
    output.append(a)
    return output
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
