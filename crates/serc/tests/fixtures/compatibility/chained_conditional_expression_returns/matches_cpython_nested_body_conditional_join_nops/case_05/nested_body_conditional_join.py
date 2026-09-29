def f(a, b, x, y, z):
    result = (
        (x if b else y)
        if a
        else z
    )
    return result
