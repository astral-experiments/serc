def f(a, b, c, d):
    if a:
        left = work()
        right = work()
    elif b:
        if c:
            if d:
                left = work()
                right = work()
            else:
                left, right = [], []
        else:
            left, right = [], []
    else:
        left, right = [], []
    return left, right
