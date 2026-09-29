def convert(outer, inner, value):
    with outer:
        if True:
            with inner:
                return value
        else:
            return value
