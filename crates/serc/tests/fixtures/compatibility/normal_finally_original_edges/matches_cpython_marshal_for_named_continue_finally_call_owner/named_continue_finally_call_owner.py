async def target(items, value, recover):
    for item in items:
        try:
            await work(item)
        except Exception as error:
            if recover:
                first()
                second()
                third()
                fourth()
                continue
            raise
        finally:
            if isinstance(value, Kind):
                value.ready = False
            if isinstance(item, str):
                record(item)
