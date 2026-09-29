def f(a, b, x, y, z):
    return (
        (x if b is None else y)
        if a
        else z
    )
