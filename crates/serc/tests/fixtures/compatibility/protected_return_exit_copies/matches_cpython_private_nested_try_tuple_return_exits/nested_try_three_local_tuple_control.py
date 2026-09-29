async def choose(value, take_outer, take_inner):
    left = 0
    right = value
    try:
        if take_outer:
            if take_inner:
                right = first(value)
            else:
                right = second(value)
        else:
            try:
                right = fallback(value)
            except ValueError:
                right = recover(value)
    except ValueError:
        right = recover_outer(value)
    return left, right, value
