async def f(items, predicate):
    try:
        from example import run
    except ImportError:
        for item in items:
            if predicate(item):
                observe(item)
    from example import run
    return await run()
