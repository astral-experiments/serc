async def f(first, second, third, work, cleanup):
    try:
        while first is not None and second is not None:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
