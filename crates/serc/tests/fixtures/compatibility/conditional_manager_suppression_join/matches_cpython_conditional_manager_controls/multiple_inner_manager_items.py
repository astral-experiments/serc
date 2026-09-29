def convert(outer, inner, extra, choose, value):
    with outer:
        if choose:
            with inner, extra:
                return value
        else:
            return value
