async def f(items, choose, load):
    for item in items:
        response = None
        try:
            mode = await choose(item)
            if mode == 1:
                return await load(item)
            elif mode == 2:
                return await load(item)
            else:
                response = await load(item)
                if item not in response.failed:
                    record(response)
        except Exception as exc:
            record(exc)
    finish()
    return None
