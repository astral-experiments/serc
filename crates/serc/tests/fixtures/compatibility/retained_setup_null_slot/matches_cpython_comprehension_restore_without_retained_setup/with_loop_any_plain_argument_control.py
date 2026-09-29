def f(manager, groups):
    with manager:
        for values in groups:
            if any(values):
                consume(values)
