def f(items):
    x = None
    try:
        x = work()
    except OuterError:
        for item in items:
            x = parse(item)
        if x is None:
            return None
    return consume(x)
