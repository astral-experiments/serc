async def f(flag, left, right):
    try:
        if flag:
            value = await left()
        else:
            value = await right()
    except ValueError:
        return 0
    else:
        return value
