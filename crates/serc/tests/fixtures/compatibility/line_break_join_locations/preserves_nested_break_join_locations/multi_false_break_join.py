def f(items, mode, first, second, third):
    for item in items:
        if mode == 0:
            if first and second:
                value = item
                break
        elif mode == 1:
            if first and second and third:
                value = item
                break
        elif mode == 2:
            if first and (second or not third):
                value = item
                break
        else:
            raise ValueError(mode)
        consume(item)
    return value
