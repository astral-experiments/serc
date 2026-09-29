def f(manager, groups):
    with manager:
        for values in groups:
            if not all(values):
                continue
            consume(values)
