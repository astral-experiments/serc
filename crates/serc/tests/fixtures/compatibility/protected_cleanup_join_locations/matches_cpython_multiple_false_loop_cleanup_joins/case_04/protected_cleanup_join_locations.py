async def f(first, second, third, work, cleanup):
    try:
        while first and second:
            await work()
            pass
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
