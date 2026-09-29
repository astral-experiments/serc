def f(flag, item):
    if flag:
        if item is None:
            value = 1.0
        else:
            value = convert(item)
        assert value
    else:
        value = 2.0
    return value
