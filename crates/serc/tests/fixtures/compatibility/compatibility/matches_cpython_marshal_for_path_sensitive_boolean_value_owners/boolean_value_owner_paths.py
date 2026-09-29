def generator_later_block(flag, value):
    if flag:
        touch()
    result = value or fallback
    yield result


def generator_sibling_yield(flag, value):
    if flag:
        yield 0
    else:
        result = value or fallback
        yield result


def generator_after_yield(first, second, third):
    yield 0
    result = first or second or third
    consume(result)


def generator_after_yield_from(items, value):
    yield from items
    result = value and fallback
    consume(result)


async def async_generator_later_block(flag, value):
    if flag:
        touch()
    result = value and fallback
    yield result


async def coroutine_sibling_await(flag, value):
    if flag:
        result = value or fallback
        consume(result)
    else:
        await source


async def coroutine_with_inner_await(manager, value):
    with manager:
        await source
        result = value or fallback
        consume(result)


async def coroutine_with_then_try(manager, value):
    with manager:
        await source
        try:
            result = value and fallback
            consume(result)
        finally:
            cleanup()


async def coroutine_try_then_with(manager, value):
    try:
        await source
        with manager:
            result = value or fallback
            consume(result)
    finally:
        cleanup()


async def coroutine_nested_with(outer, inner, value):
    with outer:
        await source
        with inner:
            result = value and fallback
            consume(result)


async def coroutine_finally(value):
    try:
        await source
    finally:
        result = value or fallback
        consume(result)
