use crate::common;

#[test]
fn async_for_split_loop_prefix() {
    common::assert_matches_cpython_marshal(
        r"async def collect(items, ready, done):
    try:
        while ready():
            async for item in items:
                await consume(item)
            if done():
                break
    except RuntimeError:
        recover()
",
        "async_for_split_loop_prefix.py",
    );
}

#[test]
fn async_for_direct_protected_tails_control() {
    common::assert_matches_cpython_marshal(
        r"async def direct_try(items):
    try:
        async for item in items:
            await consume(item)
    finally:
        cleanup()


async def direct_with(items, manager):
    with manager:
        async for item in items:
            await consume(item)


async def direct_handler(items):
    try:
        prepare()
    except RuntimeError:
        async for item in items:
            await consume(item)


async def direct_finally(items):
    try:
        prepare()
    finally:
        async for item in items:
            await consume(item)


async def finally_inside_with(items, manager):
    with manager:
        try:
            prepare()
        finally:
            async for item in items:
                await consume(item)


async def finally_inside_try(items):
    try:
        try:
            prepare()
        finally:
            async for item in items:
                await consume(item)
    except RuntimeError:
        recover()
",
        "async_for_direct_protected_tails_control.py",
    );
}

#[test]
fn async_for_branch_before_manager_exit() {
    common::assert_matches_cpython_marshal(
        r"async def collect(items, manager, selected):
    with manager:
        if selected:
            prepare()
        else:
            async for item in items:
                await consume(item)
",
        "async_for_branch_before_manager_exit.py",
    );
}

#[test]
fn et_assert_pop_else_finally_minimal() {
    common::assert_matches_cpython_marshal(
        r"def f(value):
    try:
        try:
            assert value
        except Error:
            raise
        else:
            done(value)
    finally:
        cleanup()
",
        "et_assert_pop_else_finally_minimal.py",
    );
}

#[test]
fn et_normal_try_else_assert_finally() {
    common::assert_matches_cpython_marshal(
        r"def f(manager, value):
    try:
        try:
            value = read(value)
            assert value.ready == 1
        except Error as error:
            if recover(error):
                raise
            return fallback()
        else:
            with manager:
                reset()
            return value
    finally:
        if value:
            value.close()
",
        "et_normal_try_else_assert_finally.py",
    );
}

#[test]
fn et_assert_pop_else_finally_coroutine_control() {
    common::assert_matches_cpython_marshal(
        r"async def f(value):
    try:
        try:
            assert value
        except Error:
            raise
        else:
            done(value)
    finally:
        cleanup()
",
        "et_assert_pop_else_finally_coroutine_control.py",
    );
}
