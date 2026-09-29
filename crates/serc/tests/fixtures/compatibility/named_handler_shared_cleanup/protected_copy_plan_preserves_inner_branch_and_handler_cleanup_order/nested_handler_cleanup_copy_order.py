def read(a, b, c, inner, enabled):
    left = []
    right = []
    try:
        if a:
            left.append(a)
        elif b:
            if inner:
                left.append(b)
            else:
                right.append(b)
        elif c:
            left.append(c)
    except Exception as error:
        if enabled:
            left.append(a)
        else:
            right.append(b)
    return left, right
