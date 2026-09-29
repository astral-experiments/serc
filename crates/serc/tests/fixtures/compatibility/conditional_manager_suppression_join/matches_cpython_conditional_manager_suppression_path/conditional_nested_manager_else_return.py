def convert(outer, inner, choose, value):
    with outer:
        if choose:
            return value
        else:
            with inner:
                return value
