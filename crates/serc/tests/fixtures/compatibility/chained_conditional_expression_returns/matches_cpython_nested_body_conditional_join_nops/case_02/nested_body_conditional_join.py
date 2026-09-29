def f(a, b, x, y, z):
    return (
        (x if b else y)
        if a
        else z
    )
