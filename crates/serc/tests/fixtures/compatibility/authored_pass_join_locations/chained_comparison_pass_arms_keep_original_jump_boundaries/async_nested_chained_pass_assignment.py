async def run(fetch, low, high, output):
    items = await fetch()
    previous = None
    for value in items:
        if value is not None:
            if low <= value < high:
                pass
            else:
                previous = value
        output.append(previous)
    return output
