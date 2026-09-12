use crate::common;

#[test]
fn preserves_absent_location_after_folded_with_loop_exit() {
    let source = r"async def f(manager):
    try:
        async with manager:
            while True:
                await step()
    finally:
        await finish()
";
    common::assert_matches_cpython_marshal(source, "finally_async_with_true_loop.py");
}

#[test]
fn retains_location_after_runtime_with_loop_exit() {
    let source = r"async def f(manager, running):
    try:
        async with manager:
            while running():
                await step()
    finally:
        await finish()
";
    common::assert_matches_cpython_marshal(source, "finally_async_with_runtime_loop_control.py");
}

#[test]
fn preserves_absent_location_after_folded_assertion_success() {
    let source = r"async def f(stream):
    try:
        async for value in stream:
            consume(value)
    except Error as error:
        report(error)
        assert False
    finally:
        await finish()
        await close()
";
    common::assert_matches_cpython_marshal(source, "finally_async_for_assert_false.py");
}

#[test]
fn retains_location_without_original_assertion_success() {
    let source = r"async def f(stream):
    try:
        async for value in stream:
            consume(value)
    except Error as error:
        report(error)
        raise Failure
    finally:
        await finish()
        await close()
";
    common::assert_matches_cpython_marshal(source, "finally_async_for_raise_control.py");
}

#[test]
fn shares_while_false_and_break_finally_entry() {
    let source = r"async def f(manager, running):
    async with manager:
        try:
            while running():
                value = await read()
                if value == 'stop':
                    break
                await send(value)
        finally:
            await finish()
            close()
";
    common::assert_matches_cpython_marshal(source, "finally_while_break_under_async_with.py");
}

#[test]
fn distinguishes_copyable_try_exit_from_shared_finally_exit() {
    let source = r"def protected_break(manager, active, stop):
    with manager:
        try:
            while active:
                if stop:
                    break
                step()
        except Error:
            cleanup()

def nested_finally_break(manager, active, stop):
    with manager:
        while True:
            try:
                while active:
                    if stop:
                        break
                    step()
            finally:
                cleanup()
";
    common::assert_matches_cpython_marshal(source, "protected_with_breaks_existing.py");
}
