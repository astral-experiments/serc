async def f(fetch, cleanup):
    try:
        assert (
            await fetch()
        ).value is not None
    finally:
        cleanup()
