use crate::common;

#[test]
fn protected_reraise_copies_reach_fixed_point() {
    for (filename, source) in [
        (
            "async_generator_finally_iterator_manager_break.py",
            r"async def stream(values, manager, active, asynchronous):
    try:
        yield await next_item()
    finally:
        for closer in values:
            if active:
                with manager:
                    if asynchronous:
                        await closer()
                    else:
                        closer()
                break
",
        ),
        (
            "sync_finally_iterator_manager_break.py",
            r"def run(values, manager, active):
    try:
        work()
    finally:
        for value in values:
            if active:
                with manager:
                    release(value)
                break
",
        ),
        (
            "sync_finally_iterator_manager_continue_control.py",
            r"def run(values, manager, active):
    try:
        work()
    finally:
        for value in values:
            if active:
                with manager:
                    release(value)
                continue
",
        ),
        (
            "sync_finally_iterator_manager_following_call_control.py",
            r"def run(values, manager, active):
    try:
        work()
    finally:
        for value in values:
            if active:
                with manager:
                    release(value)
                break
        finished()
",
        ),
        (
            "async_finally_conditional_except_pass.py",
            r"async def run(flag):
    try:
        await work()
    finally:
        try:
            if flag:
                await close()
        except Exception:
            pass
",
        ),
        (
            "sync_finally_conditional_except_pass.py",
            r"def run(flag):
    try:
        work()
    finally:
        try:
            if flag:
                close()
        except Exception:
            pass
",
        ),
        (
            "async_outer_iterator_finally_conditional_except_pass.py",
            r#"async def run(values, resource):
    for value in values:
        try:
            return await produce(value)
        except Exception as error:
            remember(error)
        finally:
            if resource is not None:
                closer = getattr(resource, "close", None)
                if closer is not None:
                    try:
                        result = closer()
                        if can_wait(result):
                            await result
                    except Exception:
                        pass
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copied_reraise_joins_keep_conditional_exit_locations() {
    for (filename, source) in [
        (
            "async_finally_nested_cleanup_named_second_handler.py",
            r"async def run(fut):
    try:
        return await work()
    finally:
        try:
            fut = start_cleanup()
            await finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                if fut is not None:
                    fut.cancel()
            except Exception:
                pass
        except Exception as error:
            warn(error)
",
        ),
        (
            "async_finally_nested_cleanup_unnamed_second_handler.py",
            r"async def run(fut):
    try:
        return await work()
    finally:
        try:
            fut = start_cleanup()
            await finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                if fut is not None:
                    fut.cancel()
            except Exception:
                pass
        except Exception:
            warn()
",
        ),
        (
            "async_finally_nested_cleanup_single_handler_control.py",
            r"async def run(fut):
    try:
        return await work()
    finally:
        try:
            fut = start_cleanup()
            await finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                if fut is not None:
                    fut.cancel()
            except Exception:
                pass
",
        ),
        (
            "sync_finally_nested_cleanup_named_second_handler.py",
            r"def run(fut):
    try:
        return work()
    finally:
        try:
            fut = start_cleanup()
            finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                if fut is not None:
                    fut.cancel()
            except Exception:
                pass
        except Exception as error:
            warn(error)
",
        ),
        (
            "async_finally_nested_cleanup_unconditional_cancel_control.py",
            r"async def run(fut):
    try:
        return await work()
    finally:
        try:
            fut = start_cleanup()
            await finish_cleanup(fut)
        except TimeoutError:
            warn()
            try:
                fut.cancel()
            except Exception:
                pass
        except Exception as error:
            warn(error)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
