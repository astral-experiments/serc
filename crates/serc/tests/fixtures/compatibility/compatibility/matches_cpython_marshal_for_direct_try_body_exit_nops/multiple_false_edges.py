def multiple_false_edges(first, second, name):
    try:
        if first <= 0 and second <= 0:
            raise ValueError(name)
    except TypeError:
        raise ValueError(name) from None
    return first
