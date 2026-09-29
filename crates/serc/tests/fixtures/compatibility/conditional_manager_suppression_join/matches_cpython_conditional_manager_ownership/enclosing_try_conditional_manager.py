def convert(outer, inner, choose, value):
    try:
        with outer:
            if choose:
                with inner:
                    return value
            else:
                return value
    except Error:
        return None
