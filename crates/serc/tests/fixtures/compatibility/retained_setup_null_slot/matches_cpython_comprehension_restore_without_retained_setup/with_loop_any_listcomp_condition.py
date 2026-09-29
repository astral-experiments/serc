def f(manager, groups):
    with manager:
        for values in groups:
            if any([value for value in values]):
                consume(values)
