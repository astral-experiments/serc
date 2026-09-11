use crate::common;

#[test]
fn matches_cpython_try_loop_exit_markers() {
    for header in [
        "for item in values:",
        "async for item in values:",
        "while values:",
        "while not values:",
    ] {
        for body in ["consume(item)", "if predicate(item):\n    return item"] {
            for handler in ["pass", "return None", "raise Error"] {
                let prefix = if header.starts_with("async") {
                    "async "
                } else {
                    ""
                };
                let source = format!(
                    "{prefix}def f(values):\n    try:\n        values = prepare(values)\n        {header}\n            {}\n    except Error:\n        {handler}\n    return values\n",
                    body.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "try_loop_exit_markers.py");
            }
        }
    }
}

#[test]
fn matches_cpython_shared_try_else_exit_with_following_statements() {
    let source = r"def f(values, flag):
    for value in values:
        if flag:
            if predicate(value):
                try:
                    result = read(value)
                except Error as error:
                    report(error)
                else:
                    expected = transform(value) if predicate(value) else value
                    if result != expected:
                        report(result)
        else:
            if not predicate(value):
                continue
            report(value)
        consume(value)
";
    common::assert_matches_cpython_marshal(
        source,
        "shared_try_else_exit_with_following_statements.py",
    );
}

#[test]
fn matches_cpython_for_else_cleanup_for_else_call() {
    common::assert_matches_cpython_marshal(
        r"def f(items, ready, report, recover, after):
    try:
        for item in items:
            if ready(item):
                break
        else:
            report()
    except Error:
        recover()
    after()
",
        "for_else_call.py",
    );
}

#[test]
fn matches_cpython_for_else_cleanup_for_else_outer_owner() {
    common::assert_matches_cpython_marshal(
        r"def f(items, ready, report, recover, after, cleanup):
    try:
        try:
            for item in items:
                if ready(item):
                    break
            else:
                report()
        except Error:
            recover()
        after()
    finally:
        cleanup()
",
        "for_else_outer_owner.py",
    );
}

#[test]
fn matches_cpython_for_else_cleanup_for_else_assignment_async() {
    common::assert_matches_cpython_marshal(
        r"async def f(items, ready, recover, after, cleanup):
    try:
        try:
            for item in items:
                if await ready(item):
                    break
            else:
                completed = True
        except Error:
            await recover()
        await after()
    finally:
        await cleanup()
",
        "for_else_assignment_async.py",
    );
}

#[test]
fn matches_cpython_for_else_cleanup_for_without_else_control() {
    common::assert_matches_cpython_marshal(
        r"def f(items, ready, recover, after):
    try:
        for item in items:
            if ready(item):
                break
    except Error:
        recover()
    after()
",
        "for_without_else_control.py",
    );
}
