use crate::common;

#[test]
fn matches_cpython_borrows_after_guarded_loop_recovery() {
    for scope in ["def", "async def"] {
        for guard in ["x", "not x", "x is not None"] {
            for condition in ["x", "x != y"] {
                for inner in ["except E:", "except E as error:"] {
                    for outer in ["except E:", "except (E, F) as error:"] {
                        for tail in ["use(x)", "return use(x)"] {
                            let source = format!(
                                "{scope} f(x, y):\n    try:\n        if {guard}:\n            while {condition}:\n                try:\n                    break\n                {inner}\n                    pass\n    {outer}\n        pass\n    {tail}\n"
                            );
                            common::assert_matches_cpython_marshal(
                                &source,
                                "guarded_loop_recovery_borrows.py",
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_after_surviving_while_breaks() {
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let yielded = if scope >= 2 { "    yield result\n" } else { "" };
        let tail = if scope >= 2 {
            "consume(value, result)\n    yield result"
        } else {
            "return consume(value, result)"
        };
        for body in [
            "value = next_value()\nif flag:\n    break\nif other:\n    break",
            "value = next_value()\nif flag:\n    break\nif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nconsume(value)\nif done:\n    break",
            "value = next_value()\nif flag:\n    break\nif other:\n    continue\nbreak",
            "value = next_value()\nif flag:\n    if other:\n        break\n    continue\nconsume(value)\nif done:\n    break",
            "value = next_value()\nif flag:\n    break\nelif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nif flag:\n    break\nconsume(value)",
        ] {
            let source = format!(
                "{prefix}def function(flag, other, done, result, next_value):\n{yielded}    while True:\n        {}\n    {tail}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "surviving_while_break.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_after_protected_surviving_while_breaks() {
    for (filename, source) in [
        (
            "protected_early_break_continue_call.py",
            r"def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
        (
            "protected_no_early_break_continue_call.py",
            r"def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
        (
            "protected_early_break_no_continue_call.py",
            r"def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
        (
            "protected_early_break_continue_call_async.py",
            r"async def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn protected_while_name_return_preserves_small_exit_borrowing() {
    for (filename, source) in [
        (
            "protected_infinite_loop_return.py",
            r"def collect(factory):
    items = []
    try:
        reader = factory()
        while True:
            ready, item = reader.read()
            if ready:
                items.append(item)
            else:
                break
        return items
    except Exception as error:
        pass
",
        ),
        (
            "unprotected_infinite_loop_return.py",
            r"def collect(factory):
    items = []
    reader = factory()
    while True:
        ready, item = reader.read()
        if ready:
            items.append(item)
        else:
            break
    return items
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn generator_conditional_break_reaches_nested_finally_borrows() {
    let source = r"def generator(flag, fd, acquire, release, close, timed_out, wait):
    def mark():
        remember(fd)

    acquired = False
    try:
        if flag:
            while True:
                try:
                    acquire(fd)
                    mark()
                    acquired = True
                    break
                except BlockingIOError:
                    if timed_out():
                        report(fd)
                        acquire(fd)
                        mark()
                        acquired = True
                        break
                    wait()
        else:
            while True:
                try:
                    acquire(fd)
                    mark()
                    acquired = True
                    break
                except OSError:
                    if timed_out():
                        report(fd)
                        acquire(fd)
                        mark()
                        acquired = True
                        break
                    wait()
        yield
    finally:
        try:
            if acquired:
                if flag:
                    release(fd)
                else:
                    release(fd)
        finally:
            close(fd)
";
    common::assert_matches_cpython_marshal(source, "normal_nested_finally_generator_borrow.py");
}

#[test]
fn coroutine_runtime_while_break_reaches_finally_borrows() {
    for (filename, source) in [
        (
            "async_runtime_generator_prefix_finally.py",
            r"async def lookup(client, sink):
    result = None
    try:
        cursor = None
        while result is None:
            batch = await client.fetch(cursor)
            result = next((item for item in batch.items if item.ready), None)
            next_cursor = batch.cursor
            if result is not None or next_cursor is None:
                break
            client.record(next_cursor)
            cursor = next_cursor
    except Exception as error:
        client.failed(error)
        raise
    finally:
        sink.record(client)
    return result
",
        ),
        (
            "async_runtime_loop_early_break_linear_finally.py",
            r"async def lookup(client, sink):
    result = None
    try:
        cursor = None
        while result is None:
            result, next_cursor = await client.fetch(cursor)
            if result is not None or next_cursor is None:
                break
            client.record(next_cursor)
            cursor = next_cursor
    except Exception as error:
        client.failed(error)
        raise
    finally:
        sink.record(client)
    return result
",
        ),
        (
            "async_runtime_loop_two_breaks_guarded_finally.py",
            r"async def summarize(state, client, sink):
    try:
        while state.pending or client.pending:
            remaining = state.remaining()
            if remaining <= 0:
                state.finish()
                break
            batch = await client.fetch()
            for item in batch:
                if item.ready:
                    state.record(item)
            if state.finished:
                break
    except Exception:
        client.cancel()
        raise
    finally:
        if state.finished:
            sink.record(state)
    return state.result
",
        ),
        (
            "async_loop_no_break_finally_control.py",
            r"async def lookup(client, sink):
    result = None
    try:
        while client.pending:
            result = await client.fetch()
            client.record(result)
    except Exception as error:
        client.failed(error)
    finally:
        sink.record(client)
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
