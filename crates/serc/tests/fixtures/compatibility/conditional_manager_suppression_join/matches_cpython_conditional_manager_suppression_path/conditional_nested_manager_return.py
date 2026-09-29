def convert(outer, inner, choose, work, value):
    with outer:
        if choose:
            with inner:
                result = work(value)
                return result
        else:
            result = work(value)
            return result
