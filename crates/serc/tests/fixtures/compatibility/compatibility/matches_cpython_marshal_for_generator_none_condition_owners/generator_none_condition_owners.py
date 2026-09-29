async def post_await_conditions(wait, first, second):
    if first is None:
        before()
    await wait()
    assert first is not None
    if first is None:
        use(first)
    elif second is None:
        use(second)
    if first is None and second is not None:
        use(second)
    return use(first) if second is None else fallback()


async def loop_control(wait, values):
    await wait()
    for value in values:
        if value is None:
            continue


async def try_control(wait, value):
    await wait()
    try:
        if value is None:
            use(value)
    except Error:
        recover()


async def with_control(wait, manager, value):
    await wait()
    with manager:
        if value is None:
            use(value)


async def finally_control(wait, value):
    await wait()
    try:
        use(value)
    finally:
        if value is None:
            cleanup()


def generator_conditions(first, second):
    yield before()
    if first is None and second is not None:
        yield second


def generator_while(value):
    while value is None:
        consume()
    yield value


async def async_generator_after_await(wait, value):
    await wait()
    while value is not None:
        yield value


async def async_generator_after_yield(value):
    yield value
    while value is None:
        consume()
