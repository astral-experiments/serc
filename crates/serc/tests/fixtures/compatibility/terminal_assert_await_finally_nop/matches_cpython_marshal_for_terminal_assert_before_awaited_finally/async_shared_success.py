async def f(left, right, close):
    try:
        assert left or right
    except Exception:
        raise
    finally:
        await close()
