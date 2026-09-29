def f(flag, left, right):
    try:
        if flag:
            value = left
        elif left:
            value = 2
        else:
            value = right
    except ValueError:
        return 0
    else:
        return value
