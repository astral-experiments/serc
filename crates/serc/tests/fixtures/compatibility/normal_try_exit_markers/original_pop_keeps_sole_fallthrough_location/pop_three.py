def f(flag, left, right):
    try:
        if flag:
            value = left
        else:
            value = right
    except ValueError:
        return 0
    else:
        return value
