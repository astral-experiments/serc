async def f(value, recover):
    try:
        assert value
    except Exception:
        recover()
