async def run(fetch, low, high, reject, output):
    items = await fetch()
    previous = None
    for value in items:
        if value is not None:
            if low <= value < high:
                pass
            else:
                reject(value)
                previous = value
        output.append(previous)
    return output
