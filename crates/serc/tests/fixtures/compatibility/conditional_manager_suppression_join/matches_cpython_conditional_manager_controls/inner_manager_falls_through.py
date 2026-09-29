def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                work(value)
        else:
            return value
