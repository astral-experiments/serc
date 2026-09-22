use crate::common;

#[test]
fn manager_return_preserves_inner_try_pop() {
    for (filename, source) in [
        (
            "manager-loop-inner-try.py",
            r"async def f(manager, flag, left, right):
    async with manager:
        while True:
            try:
                await prepare()
                return left if flag else right
            except Error:
                continue
",
        ),
        (
            "manager-loop-await-arm.py",
            r"async def f(manager, flag, left, right):
    async with manager:
        while True:
            try:
                await prepare()
                return await left() if flag else right
            except Error:
                continue
",
        ),
        (
            "manager-loop-nested-try.py",
            r"async def f(manager, flag, left, right):
    async with manager:
        try:
            while True:
                try:
                    return left if flag else right
                except Inner:
                    continue
        except Outer:
            raise
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn manager_return_keeps_existing_boundaries() {
    for (filename, source) in [
        (
            "manager-loop-no-join.py",
            r"async def f(manager, flag, left, right):
    async with manager:
        while True:
            try:
                await prepare()
                return left
            except Error:
                continue
",
        ),
        (
            "manager-loop-iterator.py",
            r"async def f(manager, flag, left, right):
    async with manager:
        for item in left:
            try:
                await prepare()
                return left if flag else right
            except Error:
                continue
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn finalizer_loop_preserves_original_exit() {
    for (filename, source) in [
        (
            "finalizer-linear-call.py",
            r"def f(items, consume):
    try:
        work()
    finally:
        while items:
            consume(items.pop())
    return False
",
        ),
        (
            "finalizer-existing-break.py",
            r"def f(items, consume):
    try:
        work()
    finally:
        while items:
            consume(items.pop())
            if stop:
                break
    return False
",
        ),
        (
            "finalizer-complex-following.py",
            r"def f(items, consume):
    try:
        work()
    finally:
        while items:
            consume(items.pop())
    return consume(items)
",
        ),
        (
            "finalizer-else.py",
            r"def f(items, consume):
    try:
        work()
    finally:
        while items:
            consume(items.pop())
        else:
            done()
    return False
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copied_try_return_keeps_completed_await_pop() {
    common::assert_matches_cpython_marshal(
        r"async def f(flag, left, right):
    try:
        if flag:
            value = await left()
        else:
            value = await right()
    except ValueError:
        return 0
    else:
        return value
",
        "pop_await_arms.py",
    );
}
