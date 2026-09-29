def terminal_unique_or(first, second, name):
    try:
        if not (first <= 0) or second <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return first
