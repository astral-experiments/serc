async def f(value, manager, close):
    with manager:
        try:
            assert value
        except Exception:
            raise
        finally:
            await close()
