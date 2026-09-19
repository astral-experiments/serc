use crate::common;

#[test]
fn preserves_normal_finalizer_join_locations() {
    for (filename, source) in [
        (
            "nested_protected_finally_async_generator.py",
            r"async def stream_pages(pages, token, publish):
    while True:
        try:
            stream = pages()
            try:
                async for item in stream:
                    yield item
            finally:
                try:
                    await stream.aclose()
                except Exception:
                    warning()
                if token is None and stream.version is not None:
                    await publish(stream.version)
        except LookupError as error:
            if error.args:
                raise
            continue
        token = stream.next_token
        if not token:
            return
",
        ),
        (
            "nested_normal_finally_loop_tail_control.py",
            r"async def nested_finalizer(items, selected, clean, notify):
    for item in items:
        try:
            work(item)
        finally:
            try:
                clean()
            finally:
                if selected(item):
                    await notify(item)
",
        ),
        (
            "nested_finally_authored_pass_successor_control.py",
            r"async def nested_finalizer(items, selected, clean, notify):
    for item in items:
        try:
            work(item)
        finally:
            try:
                clean()
            finally:
                if selected(item):
                    await notify(item)
        pass
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_split_handler_false_join_locations() {
    for (filename, source) in [
        (
            "handler_named_multifalse_raise_split_while_async.py",
            r"async def f(items, pending, call, enabled, retryable, save):
    for item in items:
        try:
            value = await call(item)
        except Exception as exc:
            save(exc)
            if not enabled and not retryable(exc):
                raise
        else:
            save(value)
        while pending and len(pending) > 1:
            save(pending.pop())
",
        ),
        (
            "handler_named_single_false_raise_split_while_control.py",
            r"async def f(items, pending, call, enabled, retryable, save):
    for item in items:
        try:
            value = await call(item)
        except Exception as exc:
            save(exc)
            if not enabled:
                raise
        else:
            save(value)
        while pending and len(pending) > 1:
            save(pending.pop())
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_handler_prefix_branch_join_locations() {
    for (filename, source) in [
        (
            "handler_named_partial_print_followed_loop.py",
            r"def f(items, call, enabled, details, save):
    for item in items:
        try:
            call(item)
        except Exception as exc:
            save(exc)
            if enabled:
                if details:
                    save(item)
                else:
                    save(None)
                save(exc)
        save(item)
",
        ),
        (
            "handler_named_partial_print_folded_true_control.py",
            r"def f(items, call, enabled, details, save):
    for item in items:
        try:
            call(item)
        except Exception as exc:
            save(exc)
            if True:
                if details:
                    save(item)
                else:
                    save(None)
                save(exc)
        save(item)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_final_else_handler_join_locations() {
    for (filename, source) in [
        (
            "handler_nested_else_await_finally_shared_cleanup.py",
            r"async def f(call, log, dev, enabled, finalize):
    try:
        try:
            await call()
        except BaseException:
            if dev:
                raise
            else:
                def report():
                    log()
                if enabled:
                    await log(report)
        else:
            await log(None)
    finally:
        finalize()
    return 1
",
        ),
        (
            "handler_nested_else_raise_finally_single_edge_control.py",
            r"async def f(call, log, dev, enabled, finalize):
    try:
        try:
            await call()
        except BaseException:
            if dev:
                raise
            else:
                def report():
                    log()
                if enabled:
                    raise
        else:
            await log(None)
    finally:
        finalize()
    return 1
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
