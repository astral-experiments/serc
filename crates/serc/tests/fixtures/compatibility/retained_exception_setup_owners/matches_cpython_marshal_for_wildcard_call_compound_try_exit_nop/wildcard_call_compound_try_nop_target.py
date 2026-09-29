async def f(value):
    result = None
    try:
        match value:
            case Item():
                result = await value.load()
            case _:
                reject(value)
    except Exception:
        result = "failure"
        raise
    finally:
        if result is not None:
            record(result)
