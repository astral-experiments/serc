use crate::common;

#[test]
fn nested_recovery_source_array_borrow_target() {
    let source = r"async def worker(queue, clock, record):
    while True:
        started = None
        try:
            item = await queue.get()
            if item is None:
                record('stopping')
                break
            started = clock()
            try:
                checkpoint = clock()
                await consume(item)
                record('checkpoint', clock() - checkpoint, item=item)
            except Exception as error:
                record('retry failed', error=error)
                record('retry count', item=item)
            record('completed', item=item)
        except Exception as error:
            record('failed', error=error)
        finally:
            if started is not None:
                record('duration', clock() - started)
            queue.task_done()
";
    common::assert_matches_cpython_marshal(source, "nested_recovery_source_array_borrow_target.py");
}

#[test]
fn nested_recovery_source_array_borrow_inline_control() {
    let source = r"async def worker(queue, clock, record):
    while True:
        started = None
        try:
            item = await queue.get()
            if item is None:
                record('stopping')
                break
            started = clock()
            checkpoint = clock()
            await consume(item)
            record('checkpoint', clock() - checkpoint, item=item)
            record('completed', item=item)
        except Exception as error:
            record('failed', error=error)
        finally:
            if started is not None:
                record('duration', clock() - started)
            queue.task_done()
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_recovery_source_array_borrow_inline_control.py",
    );
}

#[test]
fn matches_cpython_original_empty_return_and_copied_recovery_borrows() {
    for (filename, source) in [
        (
            "nested_pass_return_empty_borrow_join.py",
            r"def f(node, outer, middle, choice, inner, value):
    if outer:
        value = node.start(value)
        if middle:
            value = node.prepare(value)
            if choice:
                value = node.next(value)
                if inner:
                    value = node.finish(value)
                else:
                    pass
            else:
                value = node.other(value)
    else:
        value = node.fallback(value)
    return value
",
        ),
        (
            "nested_work_return_borrow_join_control.py",
            r"def f(node, outer, middle, choice, inner, value):
    if outer:
        value = node.start(value)
        if middle:
            value = node.prepare(value)
            if choice:
                value = node.next(value)
                if inner:
                    value = node.finish(value)
                else:
                    value = node.recover(value)
            else:
                value = node.other(value)
    else:
        value = node.fallback(value)
    return value
",
        ),
        (
            "named_guarded_return_copied_recovery_borrow.py",
            r"async def f(obj, reject, need_cleanup, note, cleanup):
    try:
        try:
            value = obj.read()
            note(value)
        except ValueError as error:
            note(error)
            if reject:
                note(error)
                return None
            value = None
        result = obj.dump()
        note(result)
        return result, value
    except RuntimeError:
        return None
    finally:
        if need_cleanup and obj is not None:
            note(obj)
            try:
                await cleanup(obj)
                note(obj)
            except ValueError as error:
                note(error)
                raise
",
        ),
        (
            "named_guarded_fallthrough_copied_recovery_borrow_control.py",
            r"async def f(obj, reject, need_cleanup, note, cleanup):
    try:
        try:
            value = obj.read()
            note(value)
        except ValueError as error:
            note(error)
            if reject:
                note(error)
            value = None
        result = obj.dump()
        note(result)
        return result, value
    except RuntimeError:
        return None
    finally:
        if need_cleanup and obj is not None:
            note(obj)
            try:
                await cleanup(obj)
                note(obj)
            except ValueError as error:
                note(error)
                raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_populated_and_multiline_return_borrow_boundaries() {
    for (filename, source) in [
        (
            "nested_populated_else_local_return_borrow_boundary.py",
            r"def run(outer, middle, choice, inner, value, consume):
    if outer:
        consume(value)
        if middle:
            consume(value)
            if choice:
                consume(value)
                if inner:
                    consume(value)
                else:
                    consume(value, mode=2)
            else:
                consume(value)
    else:
        consume(value)
    return value
",
        ),
        (
            "nested_multiline_call_else_local_return_borrow_boundary.py",
            r"def run(outer, middle, choice, inner, value, consume):
    if outer:
        consume(value)
        if middle:
            consume(value)
            if choice:
                consume(value)
                if inner:
                    consume(value)
                else:
                    consume(
                        value,
                        mode=2,
                    )
            else:
                consume(value)
    else:
        consume(value)
    return value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_split_loop_try_else_return_borrowing() {
    for (filename, source) in [
        (
            "split_loop_try_else_multi_false_return_borrow.py",
            r"def recover(fetch, pause, attempts, limit, wanted, error):
    for attempt in attempts:
        try:
            observed = fetch()
        except Exception as failure:
            if attempt + 1 == limit:
                raise error from failure
        else:
            if observed is not None and observed.key == wanted and observed.ready:
                return observed
        if attempt + 1 < limit:
            pause()
    raise error
",
        ),
        (
            "split_loop_try_else_single_false_return_control.py",
            r"def recover(fetch, pause, attempts, limit, wanted, error):
    for attempt in attempts:
        try:
            observed = fetch()
        except Exception as failure:
            if attempt + 1 == limit:
                raise error from failure
        else:
            if observed is not None:
                return observed
        if attempt + 1 < limit:
            pause()
    raise error
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_iterator_break_guarded_recovery_borrow() {
    let source = r#"def run(items, options, work, log, cleanup):
    failures = []
    for item in items:
        launched = False
        started = False
        finished = False
        try:
            work(item)
            launched = True
            started = True
            work(item)
            finished = True
        except (ValueError, RuntimeError) as error:
            if started and not finished:
                log("incomplete", item)
            elif not launched:
                log("unstarted", item)
            else:
                log("failed", item)
            failures.append((item, str(error)))
            log(error)
            if not options.keep_going:
                break
        finally:
            if launched and not options.keep_item:
                try:
                    cleanup(["delete", item.name])
                except (ValueError, RuntimeError) as error:
                    failures.append((item, str(error)))
                    log(error)
    return failures
"#;
    common::assert_matches_cpython_marshal(
        source,
        "named_iterator_terminal_break_guarded_recovery_borrow.py",
    );
}
