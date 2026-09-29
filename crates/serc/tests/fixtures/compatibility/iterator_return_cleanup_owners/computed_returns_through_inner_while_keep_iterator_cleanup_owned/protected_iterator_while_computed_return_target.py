async def run(items, ready, choose, produce):
    try:
        for item in items:
            result = await produce(item)
            while ready(item):
                if choose(item):
                    return produce(item)
                return result
    except Exception as error:
        raise
