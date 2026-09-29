def f(a, b, x, y, z):
    try:
        return ((x if b else y) if a else z)
    except Error:
        return z
