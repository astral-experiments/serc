def f(a, b, x, y, z):
    try:
        work()
    except Error:
        return (
            (x if b else y)
            if a
            else z
        )
    return z
