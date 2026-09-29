async def f(predicate, close):
    try:
        assert not predicate()
    except Exception:
        raise
    finally:
        await close()
