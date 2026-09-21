use crate::common;

#[test]
fn matches_cpython_borrows_after_linear_manager_finalizers() {
    for scope in 0..4 {
        let prefix = if scope == 1 || scope == 3 {
            "async "
        } else {
            ""
        };
        let terminal = if scope >= 2 {
            "yield value"
        } else {
            "return value"
        };
        for cleanup in [
            "sink.value = value",
            "other = value",
            "del sink.value",
            "del sink[value]",
            "sink.value += value",
            "sink.close(value)",
            "sink.close(value)\nsink.finish(other)",
            "sink.close(value)\nsink.finish(other)\nsink.done()",
        ] {
            let source = format!(
                "{prefix}def function(manager, source, sink, flag, value, other, values):\n    with manager:\n        try:\n            result = source.take(value)\n        finally:\n            {}\n    consume(value, other)\n    {terminal}\n",
                cleanup.replace('\n', "\n            ")
            );
            common::assert_matches_cpython_marshal(&source, "linear_manager_finally.py");
        }
    }
}

#[test]
fn preserves_cpython_manager_finalizer_block_boundaries() {
    for scope in 0..4 {
        let prefix = if scope == 1 || scope == 3 {
            "async "
        } else {
            ""
        };
        let terminal = if scope >= 2 {
            "yield value"
        } else {
            "return value"
        };
        for cleanup in [
            "sink.close(flag and value)",
            "if flag:\n    sink.close(value)",
            "while False:\n    sink.close(value)",
            "with other:\n    sink.close(value)",
            "assert True\nsink.close(value)",
            "assert False, value",
        ] {
            let source = format!(
                "{prefix}def function(manager, source, sink, flag, value, other, values):\n    with manager:\n        try:\n            result = source.take(value)\n        finally:\n            {}\n    consume(value, other)\n    {terminal}\n",
                cleanup.replace('\n', "\n            ")
            );
            common::assert_matches_cpython_marshal(&source, "manager_finally_boundaries.py");
        }
    }
}

#[test]
fn preserves_borrows_after_manager_finalizers_with_live_handlers() {
    for scope in 0..4 {
        let prefix = if scope == 1 || scope == 3 {
            "async "
        } else {
            ""
        };
        let terminal = if scope >= 2 {
            "yield value, other"
        } else {
            "return value, other"
        };
        for body in ["consume(value)", "for item in values:\n    consume(item)"] {
            for cleanup in [
                "consume(other)",
                "value = consume(other)",
                "consume(value)\nconsume(other)",
            ] {
                let source = format!(
                    "{prefix}def function(manager, value, other, values):\n    with manager:\n        try:\n            {}\n        except Error:\n            observe(value)\n        finally:\n            {}\n    consume(value, other)\n    {terminal}\n",
                    body.replace('\n', "\n            "),
                    cleanup.replace('\n', "\n            "),
                );
                common::assert_matches_cpython_marshal(&source, "manager_finally_live_handlers.py");
            }
        }
    }
}

#[test]
fn matches_cpython_original_conditional_finally_jump_owners() {
    let cases: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "fixtures/original_conditional_finally_jump_owners.json"
    ))
    .expect("public finally-exit cases are filename-to-source strings");
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(&source, &filename);
    }
}

#[test]
fn matches_cpython_async_manager_guarded_finally_break_borrows() {
    for (filename, source) in [
        (
            "async_manager_generator_guarded_finally.py",
            r"async def f(manager, state, receive):
    async with manager:
        try:
            while True:
                item = await receive()
                if item is None:
                    break
                state.record(item)
                if item.failed:
                    state.mark()
                yield item
        finally:
            if state.task is not None and not state.task.done():
                if state.queue is not None:
                    state.queue.put_nowait(None)
",
        ),
        (
            "async_manager_coroutine_guarded_finally.py",
            r"async def f(manager, state, receive, cancel):
    try:
        async with manager:
            pending = None
            other = state.other
            try:
                while True:
                    if pending is None or pending.done():
                        pending = state.start()
                    event = await receive()
                    if event.ready:
                        state.consume(pending)
                        pending = None
                    if event.stopped:
                        message = await other
                        if message is None:
                            break
                        other = state.start()
            finally:
                if pending is not None:
                    await cancel(pending)
                await cancel(other)
    except LookupError:
        state.failed()
    finally:
        if state.open:
            await state.close()
",
        ),
        (
            "no_manager_generator_control.py",
            r"async def f(state, receive):
    try:
        while True:
            item = await receive()
            if item is None:
                break
            state.record(item)
            if item.failed:
                state.mark()
            yield item
    finally:
        if state.task is not None and not state.task.done():
            if state.queue is not None:
                state.queue.put_nowait(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_sync_conditional_nested_finally_nop_owners() {
    for (filename, source) in [
        (
            "nested_finally_guard_call.py",
            r"def f(manager, enabled, work, close):
    with manager:
        try:
            work()
        finally:
            if enabled:
                try:
                    work()
                finally:
                    close()
",
        ),
        (
            "nested_finally_guard_loop.py",
            r"def f(manager, resource, values, work):
    with manager:
        try:
            work()
        finally:
            if resource is not None:
                try:
                    for value in values:
                        work(value)
                finally:
                    resource.close()
",
        ),
        (
            "ordinary_handler_guard_control.py",
            r"def f(manager, enabled, work, close):
    with manager:
        try:
            work()
        finally:
            if enabled:
                try:
                    work()
                except ValueError:
                    close()
",
        ),
        (
            "linear_guard_control.py",
            r"def f(manager, enabled, work, close):
    with manager:
        try:
            work()
        finally:
            if enabled:
                close()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_manager_finally_surviving_break_entries() {
    for (prefix, manager, work) in [
        ("def", "with", "work(value)"),
        ("def", "with", "yield value"),
        ("async def", "async with", "await work(value)"),
        ("async def", "async with", "yield value"),
    ] {
        for cleanup in [
            "manager.value = value",
            "close(value)",
            "if value:\n                value.close()",
        ] {
            let source = format!(
                "{prefix} f(manager, flag, other, value):
    {manager} manager:
        try:
            while True:
                {work}
                if flag is not None and other:
                    break
                if flag:
                    value = work(value)
                    if other:
                        break
        finally:
            {cleanup}
"
            );
            common::assert_matches_cpython_marshal(&source, "managed_finally_break_entry.py");
        }
    }
}

#[test]
fn matches_cpython_manager_finally_removed_break_entry_controls() {
    for (prefix, manager) in [("def", "with"), ("async def", "async with")] {
        for cleanup in [
            "pass",
            "manager.value = value",
            "while flag:\n                close(value)",
            "try: close(value)\n            except Exception: pass",
        ] {
            let source = format!(
                "{prefix} f(manager, flag, value):
    {manager} manager:
        try:
            while True:
                work(value)
                break
        finally:
            {cleanup}
"
            );
            common::assert_matches_cpython_marshal(&source, "managed_finally_removed_break.py");
        }
    }
}
