async def collect(load, refresh, record, finish):
    status = None
    try:
        if item := await load():
            if item.ready:
                result = await refresh(item)
                if result:
                    return result
            return item.value
    except ValueError as error:
        status = error.__class__.__name__
    finally:
        record(tags=[f"status:{'ok' if not status else 'error'}", f"detail:{status}"])
    result = await refresh(status)
    if result is None:
        record(status)
        raise ValueError(status)
    return result
