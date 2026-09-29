def f(a, b, x, y, z):
    yield None
    return (
        (x if b else y)
        if a
        else z
    )
