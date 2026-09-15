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

#[test]
fn matches_cpython_protected_while_breaks_before_small_return() {
    for (filename, source) in [
        (
            "protected_while_one_break_async_control.py",
            r"async def probe(fetch, remaining):
    collected = []
    cursor = None
    try:
        while True:
            page = await fetch(cursor)
            collected.extend(page.items)
            if remaining is not None:
                remaining -= len(page.items)
            cursor = page.next_cursor
            if cursor is None:
                break
    except Exception as error:
        report(error)
        return []
    return collected
",
        ),
        (
            "protected_while_two_breaks_async.py",
            r"async def probe(fetch, remaining):
    collected = []
    cursor = None
    try:
        while True:
            page = await fetch(cursor)
            collected.extend(page.items)
            if remaining is not None:
                remaining -= len(page.items)
                if remaining <= 0:
                    break
            cursor = page.next_cursor
            if cursor is None:
                break
    except Exception as error:
        report(error)
        return []
    return collected
",
        ),
        (
            "protected_while_two_breaks_sync.py",
            r"def probe(fetch):
    collected = []
    cursor = None
    try:
        while True:
            page = fetch(cursor)
            if page.items:
                collected.extend(page.items)
                if page.has_more:
                    cursor = page.next_cursor
                else:
                    break
            else:
                break
    except Exception as error:
        report(error)
        return []
    return collected
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_while_false_exit_location() {
    for (filename, source) in [
        (
            "folded_while_false_exit_location.py",
            r#"async def backfill(limit, fetch, write, report, finish):
    offset = 0
    try:
        while True and offset < limit:
            page = await fetch()
            if not page:
                break
            await write(page)
            offset += len(page)
            if len(page) < limit:
                report("last page")
                break
    except Exception:
        report("failed")
    finally:
        await finish()
        report("finished")
"#,
        ),
        (
            "runtime_while_false_exit_location_control.py",
            r#"async def backfill(limit, fetch, write, report, finish):
    offset = 0
    try:
        while offset < limit:
            page = await fetch()
            if not page:
                break
            await write(page)
            offset += len(page)
            if len(page) < limit:
                report("last page")
                break
    except Exception:
        report("failed")
    finally:
        await finish()
        report("finished")
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
