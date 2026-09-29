async def f(value, cleanup, recover):
    try:
        assert value
    except Exception:
        raise
    finally:
        try:
            await cleanup()
        except Exception:
            recover()
    return value
