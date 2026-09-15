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
