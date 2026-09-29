async def f(value, close):
    try:
        assert False or value
    except Exception:
        raise
    finally:
        await close()
