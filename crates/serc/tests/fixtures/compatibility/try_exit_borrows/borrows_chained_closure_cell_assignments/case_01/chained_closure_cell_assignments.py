def f(value):
    left = right = value
    return lambda: (left, right)
