use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_synchronous_with_conditions() {
    let source = r"from __future__ import annotations


def prior_if(ctx, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        value = get()
        if value:
            return use(value)


def sequential_withs(first_ctx, second_ctx, get, use):
    with first_ctx:
        first = get()
        if first:
            return use(first)
    with second_ctx:
        second = get()
        if second:
            return use(second)


def specialized_none_control(ctx, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        value = get()
        if value is not None:
            return use(value)


def same_with_marker_control(ctx, get, use, flag):
    with ctx:
        if flag:
            use(flag)
        value = get()
        if value:
            return use(value)


async def async_with_control(ctx, get, use, flag):
    if flag:
        use(flag)
    async with ctx:
        value = await get()
        if value:
            return use(value)


async def coroutine_sync_with_control(ctx, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        value = await get()
        if value:
            return use(value)


def generator_sync_with_control(ctx, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        value = get()
        if value:
            yield use(value)


def finally_then_terminal_control(ctx, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        try:
            use(flag)
        finally:
            use(flag)
        value = get()
        if value:
            return use(value)


def loop_then_terminal_control(ctx, items, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        for item in items:
            use(item)
        value = get()
        if value:
            return use(value)


def completed_finally_global_call_control(ctx, use, value, flag):
    with ctx:
        try:
            use(flag)
        finally:
            use(flag)
        if gcheck(value):
            return value


def completed_loop_global_call_control(ctx, items, use, value):
    with ctx:
        for item in items:
            use(item)
        if gcheck(value):
            return value


def terminal_inside_loop_control(ctx, items, get, use, flag):
    if flag:
        use(flag)
    with ctx:
        for item in items:
            value = get(item)
            if value:
                return use(value)
";

    common::assert_matches_cpython_marshal(
        source,
        "<terminal-synchronous-with-conditions-compatibility>",
    );
}
