def example(x, y, z):
    if x:
        return 1
    elif y:
        if z == 0:
            return 2
        elif z == 1:
            return 3
        assert False
    else:
        return 4
