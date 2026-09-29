def f(values, skip, observe):
    for value in values:
        try:
            if skip(value):
                continue
            observe(value)
        except ValueError:
            observe(None)
    return None
