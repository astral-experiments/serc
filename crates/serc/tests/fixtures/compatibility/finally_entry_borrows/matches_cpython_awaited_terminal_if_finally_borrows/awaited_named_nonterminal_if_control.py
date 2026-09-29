async def collect(load, record, finish):
    status = None
    try:
        if item := await load():
            record(item)
    except ValueError as error:
        status = error.__class__.__name__
    finally:
        record(status)
    return finish(status)
