def f(items, flag):
    x = None
    item = None
    try:
        x = work()
    except OuterError:
        while condition(flag):
            x = parse(item)
            break
        if x is None:
            return None
    return consume(x)
