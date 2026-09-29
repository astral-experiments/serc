async def f(source, make_result):
    status = None
    try:
        async for item in source:
            if item:
                status = 1
                return make_result(item=item)
        return make_result(item=None)
    finally:
        if status is None:
            status = 2
        record(status)
