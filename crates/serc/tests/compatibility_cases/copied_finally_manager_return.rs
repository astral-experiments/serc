use crate::common;

#[test]
fn matches_cpython_returns_after_copied_finally_managers() {
    for (definition, prologue) in [("def", ""), ("def", "    yield value\n"), ("async def", "")] {
        for returned in ["value", "work(value)"] {
            for prefix in ["", "        cleanup()\n"] {
                for body in ["pass", "consume(value)"] {
                    let source = format!(
                        "{definition} collect(value, manager):\n{prologue}    try:\n        return {returned}\n    finally:\n{prefix}        with manager:\n            {body}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_finally_manager_return.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_iterator_return_value_cleanup_prefix_locations() {
    for source in [
        r"def f(items, outer, inner, flag):
    with outer:
        for item in items:
            try:
                return []
            finally:
                if flag:
                    with inner:
                        cleanup()
",
        r"def f(items, outer, inner, flag):
    with outer:
        for item in items:
            try:
                return []
            finally:
                if flag:
                    cleanup()
",
        r"def f(items, outer, inner, flag):
    for item in items:
        try:
            return []
        finally:
            if flag:
                with inner:
                    cleanup()
",
    ] {
        common::assert_matches_cpython_marshal(source, "shared_iterator_cleanup_prefix.py");
    }
}

#[test]
fn constant_manager_return_cleanup_owners_match_cpython() {
    for (filename, source) in [
        (
            "constant_iterator_manager_conditional_return.py",
            r"async def example(manager, items, value):
    try:
        with manager:
            for item in items:
                try:
                    if should_return(item):
                        tick()
                        return
                except ValueError:
                    pass
    except RuntimeError:
        raise
    finally:
        if value is not None:
            finish(value)
        after()
",
        ),
        (
            "constant_named_handler_manager_await_yield_keyword.py",
            r"async def example(manager, value):
    try:
        with manager:
            try:
                work()
            except ValueError as error:
                await cancel()
                yield Event(text='failure')
                return
    finally:
        if value is not None and not value.done():
            finish(value)
        after()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copied_finally_retained_manager_return_owners_match_cpython() {
    for (filename, source) in [
        (
            "copied_finally_inner_and_outer_manager.py",
            r"def exercise(outer, inner, value):
    with outer:
        try:
            with inner():
                if value is not None:
                    work(value)
                    return
                work(None)
            after()
        except Exception as error:
            if value is not None:
                record(error)
            raise
        finally:
            if value is not None:
                finish(value)
",
        ),
        (
            "copied_finally_inner_manager_only.py",
            r"def exercise(outer, inner, value):
    try:
        with inner():
            if value is not None:
                work(value)
                return
            work(None)
        after()
    except Exception as error:
        if value is not None:
            record(error)
        raise
    finally:
        if value is not None:
            finish(value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn while_return_finally_none_owners_match_cpython() {
    for (filename, source) in [
        (
            "manager_iterator_cell_finally_target.py",
            r#"async def f(source, manager, sink, spare):
    resource = None
    try:
        resource = source.open()
        yield "ready"
        while True:
            try:
                async def receive():
                    return await resource.__anext__()
                pending = source.start(receive())
                async with manager:
                    item = await pending
            except TimeoutError:
                sink.timeout()
                yield "timeout"
                if pending is not None:
                    await source.cancel(pending)
                return
            except StopAsyncIteration:
                break
            except Exception as error:
                sink.error(error)
                break
            with sink.context():
                for value in await source.unpack(item):
                    sink.record(value)
                    yield value
                    if value.done:
                        return
    except StopAsyncIteration:
        pass
    finally:
        if resource is not None:
            await resource.aclose()
        if spare is not None:
            await source.cancel(spare)
        sink.finish()
"#,
        ),
        (
            "while_global_call_finally_target.py",
            r#"async def f(source, sink, manager, cancel_task):
    sink.attached = True
    cursor = 0
    try:
        try:
            events, terminal, latest = await source.snapshot(cursor)
        except LookupError:
            sink.gap()
            await sink.send({"type": "error", "code": "gap"})
            return
        await sink.send({"type": "ready", "cursor": cursor})
        while True:
            for event in events:
                await sink.send(event.value)
                cursor = event.cursor
            if terminal and cursor >= latest:
                record(sink, "completed")
                return
            try:
                async with manager:
                    await source.wait(cursor)
            except TimeoutError:
                await sink.heartbeat()
            try:
                events, terminal, latest = await source.snapshot(cursor)
            except LookupError:
                sink.gap()
                await sink.send({"type": "error", "code": "gap"})
                return
    except ConnectionError:
        sink.disconnected()
        raise
    finally:
        if cancel_task is not None:
            cancel_task.cancel()
            await source.join(cancel_task, return_exceptions=True)
        sink.attached = False
"#,
        ),
        (
            "while_method_call_finally_control.py",
            r#"async def f(source, sink, manager, cancel_task):
    sink.attached = True
    cursor = 0
    try:
        try:
            events, terminal, latest = await source.snapshot(cursor)
        except LookupError:
            sink.gap()
            await sink.send({"type": "error", "code": "gap"})
            return
        await sink.send({"type": "ready", "cursor": cursor})
        while True:
            for event in events:
                await sink.send(event.value)
                cursor = event.cursor
            if terminal and cursor >= latest:
                sink.complete()
                return
            try:
                async with manager:
                    await source.wait(cursor)
            except TimeoutError:
                await sink.heartbeat()
            try:
                events, terminal, latest = await source.snapshot(cursor)
            except LookupError:
                sink.gap()
                await sink.send({"type": "error", "code": "gap"})
                return
    except ConnectionError:
        sink.disconnected()
        raise
    finally:
        if cancel_task is not None:
            cancel_task.cancel()
            await source.join(cancel_task, return_exceptions=True)
        sink.attached = False
"#,
        ),
        (
            "while_global_call_break_control.py",
            r#"async def f(source, sink, manager, cancel_task):
    sink.attached = True
    cursor = 0
    try:
        try:
            events, terminal, latest = await source.snapshot(cursor)
        except LookupError:
            sink.gap()
            await sink.send({"type": "error", "code": "gap"})
            return
        await sink.send({"type": "ready", "cursor": cursor})
        while True:
            for event in events:
                await sink.send(event.value)
                cursor = event.cursor
            if terminal and cursor >= latest:
                record(sink, "completed")
                break
            try:
                async with manager:
                    await source.wait(cursor)
            except TimeoutError:
                await sink.heartbeat()
            try:
                events, terminal, latest = await source.snapshot(cursor)
            except LookupError:
                sink.gap()
                await sink.send({"type": "error", "code": "gap"})
                return
    except ConnectionError:
        sink.disconnected()
        raise
    finally:
        if cancel_task is not None:
            cancel_task.cancel()
            await source.join(cancel_task, return_exceptions=True)
        sink.attached = False
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn unnamed_handler_manager_return_owners_match_cpython() {
    for (filename, source) in [
        (
            "manager_unnamed_handler_bare_return.py",
            r"def f(manager, item):
    try:
        with manager:
            try:
                work()
            except ValueError:
                report(item)
                return
    finally:
        if item is not None:
            item.close()
",
        ),
        (
            "manager_pair_unnamed_handler_constant_return.py",
            r"def f(outer, inner, item):
    try:
        with outer:
            with inner:
                try:
                    work()
                except ValueError:
                    report(item)
                    return 0
    finally:
        if item is not None:
            item.close()
",
        ),
        (
            "manager_named_handler_bare_return_control.py",
            r"def f(manager, item):
    try:
        with manager:
            try:
                work()
            except ValueError as error:
                report(error)
                return
    finally:
        if item is not None:
            item.close()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn conditional_tuple_manager_return_owner_matches_cpython() {
    let source = r"def f(manager, item, flag):
    try:
        with manager:
            if flag is None:
                return 'missing', 'details'
            work()
        return 'ok', None
    except Exception as error:
        return 'error', str(error)
    finally:
        if item is not None and item.ready():
            item.close()
";
    common::assert_matches_cpython_marshal(source, "manager_conditional_tuple_none_return.py");
}
