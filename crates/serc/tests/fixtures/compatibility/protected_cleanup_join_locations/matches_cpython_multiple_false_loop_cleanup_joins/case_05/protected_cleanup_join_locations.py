async def f(first, second, third, work, cleanup):
    try:
        while first or second:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
