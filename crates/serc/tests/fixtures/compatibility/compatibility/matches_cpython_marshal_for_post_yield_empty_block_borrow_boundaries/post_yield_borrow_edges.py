def nested_try_exit(value):
    try:
        action()
    except Error:
        recover()
    else:
        begin()
        try:
            yield
        finally:
            finish()
    consume(value)
    return value


async def simple_async_break(items, value):
    async for item in items:
        break
    consume(value)
    return value


async def tail_conditional_async_break(items, value):
    async for item in items:
        if item:
            break
    consume(value)
    return value
