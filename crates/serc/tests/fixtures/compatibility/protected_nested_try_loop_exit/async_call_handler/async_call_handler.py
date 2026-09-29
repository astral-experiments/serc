async def f(items, outer, inner, consume, recover):
    for item in items:
        try:
            if outer(item):
                if inner(item):
                    await consume(item)
        except Exception:
            recover(item)
