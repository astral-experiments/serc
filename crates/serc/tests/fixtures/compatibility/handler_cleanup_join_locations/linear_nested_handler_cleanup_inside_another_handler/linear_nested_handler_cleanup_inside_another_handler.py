def f(primary, secondary, observe):
    try:
        primary()
    except ValueError:
        try:
            secondary()
        except TypeError:
            try:
                value = secondary()
                observe(value)
            except RuntimeError:
                pass
    observe(None)
