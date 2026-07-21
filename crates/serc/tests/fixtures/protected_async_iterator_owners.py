async def protected_finally(items, cleanup):
    try:
        async for item in items:
            yield item
    finally:
        cleanup()


async def protected_except(items, action):
    try:
        async for item in items:
            yield item
    except RuntimeError:
        action()


async def protected_except_finally(items, action, cleanup):
    try:
        async for item in items:
            yield item
    except RuntimeError:
        action()
    finally:
        cleanup()


async def protected_except_else(items, action):
    try:
        async for item in items:
            yield item
    except RuntimeError:
        action()
    else:
        action()


async def protected_leading_yield(items, cleanup):
    yield None
    try:
        async for item in items:
            yield item
    finally:
        cleanup()


async def protected_body_await(items, ready, cleanup):
    try:
        async for item in items:
            await ready(item)
            yield item
    finally:
        cleanup()


async def protected_body_yields(items, cleanup):
    try:
        async for item in items:
            yield item
            yield item
    finally:
        cleanup()


async def protected_iterator_else(items, action, cleanup):
    try:
        async for item in items:
            yield item
        else:
            action()
    finally:
        cleanup()


async def protected_nested_sync_iterator(groups, cleanup):
    try:
        async for group in groups:
            for item in group:
                yield item
    finally:
        cleanup()


async def protected_leading_action(items, action, cleanup):
    try:
        action()
        async for item in items:
            yield item
    finally:
        cleanup()


async def protected_yield_before_iterator(items, cleanup):
    try:
        yield None
        async for item in items:
            yield item
    finally:
        cleanup()


async def protected_yield_after_iterator(items, action, cleanup):
    try:
        async for item in items:
            action(item)
        yield None
    finally:
        cleanup()


async def coroutine_finally(items, action, cleanup):
    try:
        async for item in items:
            action(item)
    finally:
        cleanup()


async def no_try_control(items):
    async for item in items:
        yield item


async def leading_await_control(items, ready, cleanup):
    await ready()
    try:
        async for item in items:
            yield item
    finally:
        cleanup()


async def in_try_leading_await_control(items, ready, cleanup):
    try:
        await ready()
        async for item in items:
            yield item
    finally:
        cleanup()


async def outer_async_with_control(items, context, cleanup):
    async with context():
        try:
            async for item in items:
                yield item
        finally:
            cleanup()


async def sync_iterator_control(items, cleanup):
    try:
        for item in items:
            yield item
    finally:
        cleanup()


def generator_control(items, cleanup):
    try:
        for item in items:
            yield item
    finally:
        cleanup()
