async def f(items):
    for item in items:
        try:
            try:
                await work(item)
            except ValueError:
                continue
        except RuntimeError:
            recover()
