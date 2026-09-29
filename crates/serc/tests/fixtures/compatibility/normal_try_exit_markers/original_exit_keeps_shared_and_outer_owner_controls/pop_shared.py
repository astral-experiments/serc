def f(flag, left, right):
    value = right
    try:
        if flag:
            value = left
    except ValueError:
        return 0
    else:
        return value
