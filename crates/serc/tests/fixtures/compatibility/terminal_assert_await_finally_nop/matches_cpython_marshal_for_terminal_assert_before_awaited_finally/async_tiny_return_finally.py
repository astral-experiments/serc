async def f(value):
    try:
        assert value
    except Exception:
        raise
    finally:
        return 0
