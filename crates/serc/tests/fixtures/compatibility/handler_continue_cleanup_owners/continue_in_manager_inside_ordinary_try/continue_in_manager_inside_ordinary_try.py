def f(manager, values, skip, observe):
    for value in values:
        try:
            with manager():
                if skip(value):
                    continue
                observe(value)
        except ValueError:
            observe(None)
    return None
