
def choose(outer, inner, value, present, absent):
    with outer:
        with inner:
            result = present if value is not None else absent
    return result
