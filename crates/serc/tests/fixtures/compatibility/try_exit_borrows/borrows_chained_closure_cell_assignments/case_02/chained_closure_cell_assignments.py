def f(value):
    left = middle = right = value
    return lambda: (left, middle, right)
