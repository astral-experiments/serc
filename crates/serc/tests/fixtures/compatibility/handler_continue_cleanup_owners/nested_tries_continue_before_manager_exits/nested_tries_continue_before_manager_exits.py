def f(outer, inner, values, skip, observe):
    for value in values:
        with outer():
            with inner():
                try:
                    try:
                        if skip(value):
                            observe(value)
                            continue
                    except ValueError:
                        observe(None)
                except TypeError:
                    observe(None)
    return None
