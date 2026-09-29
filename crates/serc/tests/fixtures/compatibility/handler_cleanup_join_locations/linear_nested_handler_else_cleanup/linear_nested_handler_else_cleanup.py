def f(primary, secondary, recover, observe):
    try:
        value = primary()
    except ValueError:
        try:
            first = secondary()
            second = secondary()
        except TypeError:
            value = recover()
        else:
            value = observe(first, second)
    return value
