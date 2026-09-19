use super::super::common::assert_matches_cpython_marshal;

#[test]
fn async_cleanup_order_matches_cpython_marshal() {
    let source = r"async def sequential(first, second):
    first_result = [item async for item in first]
    second_result = [item async for item in second]
    return first_result, second_result

async def await_before(first, prepare):
    await prepare()
    return [item async for item in first]

async def await_between(first, second, prepare):
    first_result = [item async for item in first]
    await prepare()
    return [item async for item in second], first_result

async def await_after(first, prepare):
    result = [item async for item in first]
    await prepare()
    return result

async def nested(streams):
    return [[item async for item in stream] async for stream in streams]

async def nested_await(streams, transform):
    return [[await transform(item) async for item in stream] async for stream in streams]

async def loop_handler(stream, use):
    async for item in stream:
        try: use(item)
        except ValueError:
            use(None)

async def protected_loop(stream, use):
    try:
        async for item in stream:
            use(item)
    except ValueError:
        pass

async def loop_in_handler(stream, use):
    try:
        use(None)
    except ValueError:
        async for item in stream:
            use(item)

async def loop_await(stream, use):
    async for item in stream:
        await use(item)

async def genexpr_await(stream, use):
    return (await use(item) async for item in stream)

async def awaited_iterator(prepare, use):
    async for item in await prepare():
        await use(item)

async def awaited_comprehension_iterator(prepare, use):
    return [await use(item) async for item in await prepare()]

async def awaited_generator_iterator(prepare, use):
    return (await use(item) async for item in await prepare())

async def nested_generators(streams, use):
    return (await use(item) async for stream in streams async for item in stream)
";
    assert_matches_cpython_marshal(source, "async_cleanup_order.py");
}

#[test]
fn terminal_async_comprehension_restores_match_cpython_marshal() {
    let source = r"async def protected_return(stream, recover):
    try:
        return [item async for item in stream]
    except ValueError:
        recover()

async def protected_sequential_return(first, second, recover):
    try:
        initial = [item async for item in first]
        return [item async for item in second]
    except ValueError:
        recover()

async def protected_awaited_return(stream, use, recover):
    try:
        await use()
        return [item async for item in stream]
    except ValueError:
        recover()

async def protected_nested_return(streams, recover):
    try:
        return [[item async for item in stream] async for stream in streams]
    except ValueError:
        recover()
";
    assert_matches_cpython_marshal(source, "terminal_async_comprehension_restores.py");
}

#[test]
fn matches_cpython_outer_async_loop_comprehension_restore_order() {
    for (filename, source) in [
        (
            "outer_async_for_comprehension_tuple_return_target.py",
            r"async def f(pager):
    async for page in pager:
        return [item async for item in page], pager.token
    return [], None
",
        ),
        (
            "direct_async_comprehension_tuple_return_control.py",
            r"async def f(page, pager):
    return [item async for item in page], pager.token
",
        ),
        (
            "outer_async_for_comprehension_early_return_target.py",
            r"async def f(pages, finish):
    try:
        async for page in pages:
            items = [item async for item in page]
            if not items:
                return
            for item in items:
                yield item
    finally:
        finish()
",
        ),
        (
            "direct_async_comprehension_early_return_control.py",
            r"async def f(page, finish):
    try:
        items = [item async for item in page]
        if not items:
            return
        for item in items:
            yield item
    finally:
        finish()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
