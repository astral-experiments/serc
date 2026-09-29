async def collect(mode, pending, fetch, stop):
    result = []
    if mode:
        while pending():
            value = await fetch()
            result.append(value)
            if stop(value):
                break
    else:
        result.append(await fetch())
    return result
