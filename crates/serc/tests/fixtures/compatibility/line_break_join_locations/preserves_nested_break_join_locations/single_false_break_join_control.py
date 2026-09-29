def f(items, mode, first, second):
    for item in items:
        if mode == 0:
            if first:
                break
        elif mode == 1:
            if first or second:
                break
        else:
            raise ValueError(mode)
        consume(item)
    return None
