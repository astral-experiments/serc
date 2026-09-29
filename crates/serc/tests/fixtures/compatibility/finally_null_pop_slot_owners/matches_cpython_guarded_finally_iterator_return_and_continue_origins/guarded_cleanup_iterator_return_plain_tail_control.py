async def f(items, first, second):
    try:
        for item in items:
            try:
                return item
            finally:
                cleanup(item)
    except Exception:
        raise
