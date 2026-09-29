def terminal_compare(value, name):
    try:
        if value <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return value
