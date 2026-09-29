async def f(first, second, third, work, cleanup):
    try:
        while first and second:
            await work()
        else:
            done()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
