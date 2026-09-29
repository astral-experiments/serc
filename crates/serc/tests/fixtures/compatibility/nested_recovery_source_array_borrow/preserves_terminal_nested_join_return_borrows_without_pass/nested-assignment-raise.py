def f(value, outer, first, second, choose, fallback):
    if outer:
        if first:
            value = update(value)
        elif second:
            if choose:
                value = left(value)
            elif fallback:
                value = right(value)
            else:
                raise ValueError("no value")
    elif second:
        if choose:
            value = left(value)
        else:
            raise ValueError("no value")
    return value
