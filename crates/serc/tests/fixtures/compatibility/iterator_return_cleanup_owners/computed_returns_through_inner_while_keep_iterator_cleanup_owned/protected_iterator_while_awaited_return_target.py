async def run(items, ready, produce):
    try:
        for item in items:
            while ready(item):
                return await produce(item)
    except Exception as error:
        raise
