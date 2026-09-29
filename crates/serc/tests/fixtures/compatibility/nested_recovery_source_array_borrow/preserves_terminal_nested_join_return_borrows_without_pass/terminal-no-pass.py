def f(value, outer, first, second):
    if outer:
        if first:
            return update(value)
        elif second:
            return convert(value)
    else:
        value = other(value)
    return value
