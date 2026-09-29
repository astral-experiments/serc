def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                return value
        else:
            return value
        work(value)
