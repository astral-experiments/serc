use crate::common::assert_matches_cpython_marshal;
use std::fmt::Write as _;

#[test]
fn matches_cpython_copied_finally_reusing_consecutive_pop_block_slots() {
    for asynchronous in [false, true] {
        for returned in ["value", "work(value)"] {
            for handler in [
                "except Error:\n        raise",
                "except Error as error:\n        return error",
            ] {
                for condition in ["flag", "flag is None", "flag is not None"] {
                    let source = format!(
                        "{}def f(flag, value):\n    try:\n        {}work(value)\n        return {returned}\n    {handler}\n    finally:\n        if {condition}:\n            finish()\n",
                        if asynchronous { "async " } else { "" },
                        if asynchronous { "await " } else { "" },
                    );
                    assert_matches_cpython_marshal(&source, "consecutive_return_pop_blocks.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_first_finally_markers_reusing_pop_block_slots() {
    for kind in ["sync", "coroutine", "generator", "async_generator"] {
        let asynchronous = matches!(kind, "coroutine" | "async_generator");
        for prefix in [
            "        result = 1 if flag else 2\n",
            "        if flag:\n            result = 1\n        else:\n            result = 2\n",
        ] {
            for condition in ["value", "value is None", "value is not None"] {
                for calls in 2..=4 {
                    let mut source = format!(
                        "{}def f(value, flag):\n",
                        if asynchronous { "async " } else { "" },
                    );
                    if matches!(kind, "generator" | "async_generator") {
                        source.push_str("    yield value\n");
                    }
                    source.push_str("    try:\n");
                    source.push_str(prefix);
                    for _ in 0..calls {
                        source.push_str("        work()\n");
                    }
                    writeln!(
                        source,
                        "    finally:\n        if {condition}:\n            {}value.close()",
                        if asynchronous { "await " } else { "" },
                    )
                    .unwrap();
                    assert_matches_cpython_marshal(
                        &source,
                        &format!("finally_null_pop_{kind}_{calls}_{condition}.py"),
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_finally_owners_after_folded_branch_joins() {
    for (index, source) in [
        r"def f(value, flag, other, task, manager):
    with manager:
        try:
            result = 1 if flag else 2
            work()
            work()
            work()
            if False:
                task(value)
        finally:
            if value:
                task(value)
    after()
",
        r"async def f(value, flag, other, task, manager):
    await prepare()
    try:
        result = 1 if flag else 2
        work()
        work()
        work()
        if False:
            task(value)
    finally:
        if value:
            await value.close()
    after()
",
        r"def f(value, flag, other, task, manager):
    try:
        try:
            result = 1 if flag else 2
            work()
            work()
            work()
            if False:
                task(value)
        finally:
            if (
                value
                is
                None
            ):
                manager.finish(value)
    finally:
        if other:
            task(value)
    after()
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_marshal(source, &format!("finally_folded_join_{index}.py"));
    }
}

#[test]
fn matches_cpython_finally_owners_after_constant_branch_joins() {
    for branch in [
        "if True:\n                pass",
        "if True:\n                task(value)",
    ] {
        let source = format!(
            r"def f(value, flag, task, manager):
    with manager:
        try:
            result = 1 if flag else 2
            work()
            work()
            work()
            {branch}
        finally:
            if value:
                task(value)
    after()
",
        );
        assert_matches_cpython_marshal(&source, "finally_constant_branch_join.py");
    }
}

#[test]
fn matches_cpython_nested_finally_markers_after_collection_folding() {
    for collection in [
        "[1, 2, 3]",
        "[1, 2, 3, 4]",
        "[1, 2, 3, 4, 5]",
        "{1, 2, 3}",
        "{1, 2, 3, 4}",
        "{1, 2, 3, 4, 5}",
        "(1,)",
        "(1, 2)",
        "(1, 2, 3)",
    ] {
        let source = format!(
            "def f(flag):
    try:
        try:
            consume({collection})
        finally:
            if flag:
                work()
    finally:
        finish()
"
        );
        assert_matches_cpython_marshal(&source, "nested_finally_collection_fold.py");
    }
}

#[test]
fn async_iterator_returns_preserve_ordinary_try_null_pop_slots() {
    for (filename, source) in [
        (
            "coroutine_async_iterator_bare_return_finally_none_owner.py",
            r"async def f(source, open_result, stop, close, idle):
    while True:
        result = None
        try:
            result = await open_result()
            async for item in source:
                if stop(item):
                    return
        except Exception as error:
            record(error)
        finally:
            if result is not None:
                close(result)
        await idle()
",
        ),
        (
            "coroutine_async_iterator_handlerless_finally_control.py",
            r"async def f(source, make_result):
    status = None
    try:
        async for item in source:
            if item:
                status = 1
                return make_result(item=item)
        return make_result(item=None)
    finally:
        if status is None:
            status = 2
        record(status)
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn async_iterator_value_returns_capture_finally_null_pop_slots() {
    let source = r"async def f(source, prepare, stop, process, cleanup):
    reload = False
    status = None
    try:
        if await prepare():
            status = Result.SKIPPED
            return Finish(retry=True, reload=reload)
        accepted = True
        async for item in source:
            process(item)
            if stop(item):
                status = (
                    Result.SUCCESS
                    if accepted
                    else Result.ERROR
                )
                return Finish(
                    retry=False, reload=reload
                )
            if item.failed:
                accepted = False
        return Finish(retry=True, reload=reload)
    except Exception:
        status = Result.ERROR
        raise
    finally:
        if status is None:
            status = Result.MISSING
        record(status.value)
        cleanup(status)
";
    assert_matches_cpython_marshal(
        source,
        "coroutine_async_iterator_branched_call_return_finally_none_owner.py",
    );
}

#[test]
fn coroutine_return_cleanup_preserves_original_finally_none_slots() {
    for (filename, source) in [
        (
            "coroutine_iterator_handler_return_none_finalizer.py",
            r#"async def f(manager, handle, attempts):
    try:
        for attempt in range(attempts):
            try:
                await work()
            except Exception:
                if should_stop(attempt):
                    record("abandoned", attempt, handle)
                    return
                if await cancelled():
                    return
                continue
            record("success", attempt, handle)
            return
    finally:
        if handle is not None:
            async with manager():
                await release(handle)
"#,
        ),
        (
            "coroutine_iterator_handler_linear_finalizer_control.py",
            r#"async def f(manager, handle, attempts):
    try:
        for attempt in range(attempts):
            try:
                await work()
            except Exception:
                if should_stop(attempt):
                    record("abandoned", attempt, handle)
                    return
                if await cancelled():
                    return
                continue
            record("success", attempt, handle)
            return
    finally:
        async with manager():
            await release(handle)
"#,
        ),
        (
            "coroutine_manager_handler_return_none_finalizer.py",
            r"from contextlib import suppress

async def f(manager, handle, flag):
    try:
        await work()
        with manager():
            if flag:
                return
            record(handle)
    except Exception as error:
        with manager():
            if flag:
                record(error)
                return
            report(error)
    finally:
        if handle is not None:
            handle.cancel()
            with suppress(Exception):
                await handle
",
        ),
        (
            "coroutine_manager_handler_linear_finalizer_control.py",
            r"from contextlib import suppress

async def f(manager, handle, flag):
    try:
        await work()
        with manager():
            if flag:
                return
            record(handle)
    except Exception as error:
        with manager():
            if flag:
                record(error)
                return
            report(error)
    finally:
        handle.cancel()
        with suppress(Exception):
            await handle
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn iterator_handler_continue_preserves_nested_finalizer_setup_slot() {
    for (filename, source) in [
        (
            "iterator_value_return_nested_try_finalizer.py",
            r"def f(values, make):
    for value in values:
        resource = None
        try:
            resource = make(value)
            resource.prepare()
            resource.bind(value)
            return value
        except OSError:
            continue
        finally:
            try:
                if resource is not None:
                    resource.close()
            except Exception:
                pass
    return None
",
        ),
        (
            "iterator_value_return_plain_finalizer_control.py",
            r"def f(values, make):
    for value in values:
        resource = None
        try:
            resource = make(value)
            resource.prepare()
            resource.bind(value)
            return value
        except OSError:
            continue
        finally:
            if resource is not None:
                resource.close()
    return None
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
