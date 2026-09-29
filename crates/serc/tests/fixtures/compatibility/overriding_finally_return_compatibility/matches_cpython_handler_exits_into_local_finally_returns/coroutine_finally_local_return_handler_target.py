async def f(work, value):
    try:
        value = await work()
        record(value)
    except Exception:
        report()
    finally:
        return value
