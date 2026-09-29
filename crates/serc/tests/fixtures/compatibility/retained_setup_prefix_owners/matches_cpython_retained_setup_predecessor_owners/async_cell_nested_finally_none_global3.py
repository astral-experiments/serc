async def f(resource, items, skip):
    tasks = (resource for item in items)
    try:
        if skip:
            return
        tick_1()
        tick_2()
        tick_3()
    finally:
        try:
            if resource is not None:
                await resource.close()
        except Exception:
            pass
