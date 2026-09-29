async def f(value, close):
    try:
        yield value
        pass
    finally:
        await close()
        await close()
