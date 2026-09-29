async def f(first, second, third, work, cleanup):
    try:
        while first and second and third:
            await work()
    except LookupError:
        raise
    except Exception:
        raise
    finally:
        await cleanup()
