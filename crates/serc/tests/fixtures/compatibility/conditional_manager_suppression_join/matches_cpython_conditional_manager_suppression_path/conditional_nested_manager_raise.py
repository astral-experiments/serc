def convert(outer, inner, choose, error, value):
    with outer:
        if choose:
            with inner:
                raise error
        else:
            return value
