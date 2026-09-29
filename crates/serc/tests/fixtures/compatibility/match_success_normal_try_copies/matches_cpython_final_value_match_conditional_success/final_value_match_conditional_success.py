async def f(value, work, finish):
    await work(value)
    try:
        match value:
            case Kind.FIRST:
                if value.enabled:
                    left, right = work(value)
                else:
                    left, right = work(value)
            case Kind.LAST:
                left, right = work(value)
    except ValueError as error:
        raise RuntimeError(value) from error
    return finish(left, right, value)
