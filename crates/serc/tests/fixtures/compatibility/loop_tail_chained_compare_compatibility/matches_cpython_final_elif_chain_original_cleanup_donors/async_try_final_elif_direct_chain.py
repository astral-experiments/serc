async def run(fetch, previous, current, output):
    try:
        items = await fetch()
        for limit in items:
            if previous < limit <= current:
                output.append(limit)
            elif current < limit <= previous:
                output.append(limit)
    except Exception:
        output.clear()
