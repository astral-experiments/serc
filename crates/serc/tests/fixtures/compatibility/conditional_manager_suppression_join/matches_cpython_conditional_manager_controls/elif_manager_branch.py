def convert(outer, inner, first, second, value):
    with outer:
        if first:
            return value
        elif second:
            with inner:
                return value
        else:
            return value
