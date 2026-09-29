async def coroutine(items):
    for item in items:
        assert item
        consume(item)


async def negated_coroutine(items):
    for item in items:
        assert not item
        consume(item)


def generator(items):
    for item in items:
        assert item
        yield item


async def async_generator(items):
    for item in items:
        assert item
        yield item


async def asynchronous_iterator(items):
    async for item in items:
        assert item
        consume(item)


async def awaited_control(items):
    for item in items:
        assert await predicate(item)
        consume(item)


def ordinary_control(items):
    for item in items:
        assert item
        consume(item)


async def terminal_control(items):
    for item in items:
        consume(item)
        assert item


async def nested_control(items):
    for item in items:
        if predicate(item):
            assert item
            consume(item)
        after(item)


async def while_control(items):
    while items is not None:
        assert items
        consume(items)
        break
