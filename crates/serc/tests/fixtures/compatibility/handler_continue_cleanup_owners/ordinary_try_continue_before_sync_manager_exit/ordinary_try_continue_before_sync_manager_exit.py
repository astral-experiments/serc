def f(manager, values, skip, observe):
    for value in values:
        with manager():
            try:
                observe(value)
                if skip(value):
                    continue
            except ValueError:
                observe(None)
    return None
