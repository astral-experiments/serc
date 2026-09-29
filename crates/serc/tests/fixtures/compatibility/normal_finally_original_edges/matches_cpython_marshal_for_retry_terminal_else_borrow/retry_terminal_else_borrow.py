async def target(item):
    try:
        value = await load(item)
    except Failure:
        error = explain()
        for attempt in range(2):
            try:
                value = await retry(item)
                yield value
                break
            except Failure:
                error = explain()
        else:
            yield error
            return
    yield pack(item.first, item.second, item.third, item.fourth, item.fifth)
