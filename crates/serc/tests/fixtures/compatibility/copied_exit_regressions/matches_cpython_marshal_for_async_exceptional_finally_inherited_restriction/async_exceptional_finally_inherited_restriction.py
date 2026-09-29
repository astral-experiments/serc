async def target(work, cleanup, after, finish):
    try:
        try:
            await work()
        finally:
            try:
                await cleanup()
            except Exception:
                after()
        after()
    except Exception:
        pass
    finally:
        finish()
