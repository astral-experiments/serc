def f(manager, groups):
    with manager:
        for values in groups:
            if not all([value for value in values]):
                continue
            consume(values)
