def f(a, b, x, y, z):
    while a:
        result = ((x if b else y) if a else z)
        break
    return result
