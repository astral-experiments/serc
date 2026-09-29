def f(items):
    x = None
    try:
        x = work()
    except OuterError:
        for item in items:
            try:
                x = parse(item)
            except InnerError:
                x = None
    return consume(x)
