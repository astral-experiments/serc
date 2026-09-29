async def f(left, right):
    try:
        x = first(left)
        y = second(right)
    except Error as exc:
        return failure("first", str(exc))
    except OtherError:
        return failure("second", "unavailable")
    if not checker.same(x, y):
        return failure("third", "different")
    return await finish(left)
