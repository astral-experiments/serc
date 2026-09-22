use crate::common;

#[test]
fn matches_cpython_general_raise_bool() {
    let source = r"def probe(action, first, second):
    try:
        action()
    except Exception as error:
        if first and second:
            raise
";
    common::assert_matches_cpython_marshal(source, "general_raise_bool.py");
}

#[test]
fn matches_cpython_general_prefixed_raise_bool() {
    let source = r"async def probe(action, lookup, first):
    try:
        await action()
    except Exception as error:
        current = await lookup()
        if current and first:
            raise RuntimeError() from error
";
    common::assert_matches_cpython_marshal(source, "general_prefixed_raise_bool.py");
}

#[test]
fn matches_cpython_specialized_assignment_bool() {
    let source = r"def probe(action, state, expected):
    try:
        action()
    except Exception as error:
        if not state.closed and state.generation == expected:
            state.error = error
";
    common::assert_matches_cpython_marshal(source, "specialized_assignment_bool.py");
}

#[test]
fn matches_cpython_specialized_nested_arm() {
    let source = r"def probe(action, state, emit):
    try:
        action()
    except Exception as error:
        if isinstance(error, ValueError):
            emit(error)
            if not state.done():
                state.finish(None)
        else:
            emit(error)
";
    common::assert_matches_cpython_marshal(source, "specialized_nested_arm.py");
}

#[test]
fn matches_cpython_control_single_false_raise() {
    let source = r"def probe(action, first):
    try:
        action()
    except Exception as error:
        if first:
            raise
";
    common::assert_matches_cpython_marshal(source, "control_single_false_raise.py");
}

#[test]
fn matches_cpython_control_authored_pass() {
    let source = r"def probe(action, first, second):
    try:
        action()
    except Exception as error:
        if first and second:
            raise
        pass
";
    common::assert_matches_cpython_marshal(source, "control_authored_pass.py");
}

#[test]
fn matches_cpython_named_handler_terminal_while_break() {
    for (filename, source) in [
        (
            "named_handler_terminal_while_break.py",
            r"async def consume(queue, process, fail):
    pending = []
    closed = False
    while pending or not closed:
        try:
            await process()
        except Exception as error:
            fail(error)
            pending.clear()
            while True:
                try:
                    queued = queue.get_nowait()
                except LookupError:
                    break
                if queued is None:
                    closed = True
                else:
                    fail(queued, error)
",
        ),
        (
            "named_handler_terminal_while_pass_control.py",
            r"async def consume(queue, process, fail):
    pending = []
    closed = False
    while pending or not closed:
        try:
            await process()
        except Exception as error:
            fail(error)
            pending.clear()
            while True:
                try:
                    queued = queue.get_nowait()
                except LookupError:
                    break
                if queued is None:
                    closed = True
                else:
                    fail(queued, error)
            pass
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_named_handler_false_cleanup_locations() {
    for (filename, source) in [
        (
            "nested_named_handler_false_cleanup.py",
            r#"async def run(value, request, retry, report, lookup):
    try:
        await request(value)
    except TypeError as outer:
        message = str(outer)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            await retry(value)
        except Exception as inner:
            await report(inner)
            state = await lookup(value)
            if not isinstance(state, State) or state.status != "done":
                await report(state)
    except Exception as outer:
        state = await lookup(value)
        if not isinstance(state, State) or state.status != "done":
            await report(state)
"#,
        ),
        (
            "nested_named_handler_sync_false_cleanup.py",
            r#"def run(value, request, retry, report, lookup):
    try:
        request(value)
    except TypeError as outer:
        message = str(outer)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            retry(value)
        except Exception as inner:
            report(inner)
            state = lookup(value)
            if not isinstance(state, State) or state.status != "done":
                report(state)
    except Exception as outer:
        state = lookup(value)
        if not isinstance(state, State) or state.status != "done":
            report(state)
"#,
        ),
        (
            "nested_unnamed_outer_handler_false_cleanup.py",
            r#"async def run(value, request, retry, report, lookup):
    try:
        await request(value)
    except TypeError:
        message = str(value)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            await retry(value)
        except Exception as inner:
            await report(inner)
            state = await lookup(value)
            if not isinstance(state, State) or state.status != "done":
                await report(state)
    except Exception:
        state = await lookup(value)
        if not isinstance(state, State) or state.status != "done":
            await report(state)
"#,
        ),
        (
            "nested_named_handler_explicit_none_return.py",
            r#"async def run(value, request, retry, report, lookup):
    try:
        await request(value)
    except TypeError as outer:
        message = str(outer)
        if "retry" not in message or "supported" not in message:
            raise
        try:
            await retry(value)
        except Exception as inner:
            await report(inner)
            state = await lookup(value)
            if not isinstance(state, State) or state.status != "done":
                await report(state)
    except Exception as outer:
        state = await lookup(value)
        if not isinstance(state, State) or state.status != "done":
            await report(state)
    return None
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_handler_complete_else_cleanup_joins() {
    for (filename, source) in [
        (
            "named_complete_else_terminal_call.py",
            r"def test(outer, inner):
    try:
        work()
    except Error as error:
        if outer:
            first(error)
        else:
            if inner:
                second()
            else:
                third()
",
        ),
        (
            "named_complete_else_loop_value_join.py",
            r"def test(items, limit, random_mode):
    count = 0
    success = False
    while count <= limit and not success:
        try:
            work()
            success = True
        except Error as error:
            count += 1
            if count > limit:
                raise RuntimeError() from error
            else:
                wait()
                if random_mode:
                    remove(items)
                    value = choose(items) if items else None
                else:
                    value = items.pop() if items else None
    done()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_terminal_named_handler_original_false_joins() {
    for condition in ["flag and error.code", "flag or error.code", "flag"] {
        let source = format!(
            "def f(flag):\n    try:\n        work()\n        return\n    except ValueError as error:\n        if {condition}:\n            notify()\n            return\n    except Exception as error:\n        raise error\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_named_handler_false_join.py");
    }
}

#[test]
fn matches_cpython_named_handler_runtime_break_return_copies() {
    for (function, poll) in [("def", "poll()"), ("async def", "await poll()")] {
        for returned in ["first", "first, second", "None"] {
            let source = format!(
                "{function} f(flag):\n    first = []\n    second = []\n    try:\n        work()\n    except Exception as error:\n        while flag:\n            events = {poll}\n            if not events:\n                break\n            first.append(events)\n    return {returned}\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_break_return_copy.py");
        }
    }
}
