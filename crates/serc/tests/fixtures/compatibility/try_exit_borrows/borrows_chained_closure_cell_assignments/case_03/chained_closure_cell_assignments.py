def f(value, flag):
    if flag:
        left = right = value
    else:
        left, right = value
    return lambda: (left, right)
