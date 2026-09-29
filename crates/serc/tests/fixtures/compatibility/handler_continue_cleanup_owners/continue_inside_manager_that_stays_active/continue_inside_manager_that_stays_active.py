def f(manager, values, skip, observe):
    with manager():
        for value in values:
            try:
                if skip(value):
                    continue
                observe(value)
            except ValueError:
                observe(None)
    return None
