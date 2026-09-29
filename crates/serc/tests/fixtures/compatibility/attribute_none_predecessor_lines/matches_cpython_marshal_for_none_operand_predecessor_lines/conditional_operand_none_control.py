def f(flag, left, right):
    if (
        left()
        if flag
        else right()
    ) is None:
        return 1
    return 2
