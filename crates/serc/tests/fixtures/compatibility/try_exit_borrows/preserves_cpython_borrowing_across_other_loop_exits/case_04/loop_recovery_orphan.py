def f(items, flag):
    x = None
    item = None
    try:
        x = work()
    except OuterError:
        while True:
            try:
                x = parse(item)
            except InnerError:
                x = None
        if x is None:
            return None
    return consume(x)
