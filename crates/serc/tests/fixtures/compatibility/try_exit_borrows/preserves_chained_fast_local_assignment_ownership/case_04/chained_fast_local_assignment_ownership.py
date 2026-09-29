def f(value):
    left = right = value
    return right, lambda: left
