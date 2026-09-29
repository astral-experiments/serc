def convert(outer, choose, value):
    with outer:
        if choose:
            return value
        else:
            raise Error
