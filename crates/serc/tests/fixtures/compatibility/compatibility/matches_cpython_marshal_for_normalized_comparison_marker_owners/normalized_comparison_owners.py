async def coroutine():
    await source
    if test == expected:
        await other


def generator(value):
    while value > 0:
        yield value
        value -= 1
