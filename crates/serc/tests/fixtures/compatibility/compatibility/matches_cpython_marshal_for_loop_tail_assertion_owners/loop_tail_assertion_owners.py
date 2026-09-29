async def coroutine_direct(wait, items, value):
    await wait()
    for item in items:
        assert value is None


def generator_or(items, value, other):
    yield value
    for item in items:
        assert value is None or other


async def async_generator_and(items, first, second):
    yield first
    for item in items:
        assert first is None and second


async def protected_try(wait, items, value):
    await wait()
    try:
        for item in items:
            assert value is None
    except Error:
        recover()


async def protected_with(wait, items, value, manager):
    await wait()
    with manager:
        for item in items:
            assert value is None or item


async def protected_finally(wait, items, value):
    try:
        await wait()
    finally:
        for item in items:
            assert value is None
