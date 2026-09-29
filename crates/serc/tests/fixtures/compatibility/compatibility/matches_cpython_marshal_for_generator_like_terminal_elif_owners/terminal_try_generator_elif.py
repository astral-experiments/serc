def generator_elif(first, second):
    yield 1
    try:
        if first:
            one()
        elif second:
            two()
    except Exception:
        recover()

async def async_generator_elif(first, second):
    yield 1
    try:
        if first:
            one()
        elif second:
            two()
    except Exception:
        recover()

async def coroutine_after_await(first, second, final):
    await before()
    try:
        if first:
            one()
        elif second:
            two()
        if final:
            three()
    except Exception:
        recover()
