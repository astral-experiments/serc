def f(a, b, x, y, z):
    with manager():
        return ((x if b else y) if a else z)
