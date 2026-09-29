async def run(fetch, upper, output):
    items = await fetch()
    if items:
        for value in items:
            if value is None:
                output.append(value)
            elif not 0 <= value <= upper:
                output.append(value)
    return output
