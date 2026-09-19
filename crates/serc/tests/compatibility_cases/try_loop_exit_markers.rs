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

#[test]
fn matches_cpython_loop_break_normal_try_exit_nops() {
    for (filename, source) in [
        (
            "try_else_sync_for_break_exhaustion_assign_exit_nop.py",
            r"def f(items, choose, prepare, finish):
    result = None
    try:
        prepare()
    except ValueError:
        result = None
    else:
        for item in items:
            if choose(item):
                result = item
                break
        else:
            result = tuple(items)
    finish(result)
    return result
",
        ),
        (
            "coroutine_try_else_sync_for_break_empty_exhaustion_exit_nop.py",
            r"async def f(items, prepare, recover, finish):
    result = None
    try:
        await prepare()
    except ValueError as exc:
        result = str(exc)
    else:
        for item in items:
            result = await recover(item)
            if result:
                break
    await finish(result)
    return result
",
        ),
        (
            "try_else_sync_for_no_break_exit_nop_control.py",
            r"def f(items, choose, prepare, finish):
    result = None
    try:
        prepare()
    except ValueError:
        result = None
    else:
        for item in items:
            result = choose(item)
        else:
            result = tuple(items)
    finish(result)
    return result
",
        ),
        (
            "try_else_sync_for_break_following_work_exit_nop_control.py",
            r"def f(items, choose, prepare, finish):
    result = None
    try:
        prepare()
    except ValueError:
        result = None
    else:
        for item in items:
            if choose(item):
                result = item
                break
        else:
            result = tuple(items)
        finish(result)
    finish(result)
    return result
",
        ),
        (
            "runtime_while_break_else_assignment_conditional_finally_exit_nop.py",
            r#"def example(active, choose, prepare, recover, finish, guard):
    result = 1
    try:
        prepare()
        while active():
            result = prepare()
            if choose(result):
                break
        else:
            recover("done" if guard else "empty")
            result = 0
    except ValueError as error:
        recover(error)
        result = 2
    finally:
        if guard is not None:
            finish(result)
    return result
"#,
        ),
        (
            "runtime_while_break_else_assignment_following_work_control.py",
            r#"def example(active, choose, prepare, recover, finish, guard):
    result = 1
    try:
        prepare()
        while active():
            result = prepare()
            if choose(result):
                break
        else:
            recover("done" if guard else "empty")
            result = 0
        prepare()
    except ValueError as error:
        recover(error)
        result = 2
    finally:
        if guard is not None:
            finish(result)
    return result
"#,
        ),
        (
            "constant_while_break_else_assignment_finally_control.py",
            r#"def example(active, choose, prepare, recover, finish, guard):
    result = 1
    try:
        prepare()
        while True:
            result = prepare()
            if choose(result):
                break
        else:
            recover("done" if guard else "empty")
            result = 0
    except ValueError as error:
        recover(error)
        result = 2
    finally:
        if guard is not None:
            finish(result)
    return result
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
