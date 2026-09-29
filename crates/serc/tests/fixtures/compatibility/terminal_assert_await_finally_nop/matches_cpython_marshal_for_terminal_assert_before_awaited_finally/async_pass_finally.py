async def f(value):
    try:
        assert value
    except Exception:
        raise
    finally:
        pass
