async def f(value, close, recover):
    try:
        assert value
    except ValueError:
        recover()
    finally:
        await close()
