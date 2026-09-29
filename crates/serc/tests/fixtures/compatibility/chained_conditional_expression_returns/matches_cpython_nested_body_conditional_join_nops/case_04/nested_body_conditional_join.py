def f(a, b, x, y, z):
    return consume(value=(
        (x if b else y)
        if a
        else z
    ))
