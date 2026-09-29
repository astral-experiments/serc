def f(value):
    left = right = value
    return left, lambda: right
