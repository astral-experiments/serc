def probe(marker, low, high, bound, choice, flag, left, right, obj, make):
    if marker is not None:
        if low >= bound:
            scratch = left
        else:
            scratch = right
        if low != high:
            scratch = right
        value = obj.payload if True else right
        if flag:
            result = None
        else:
            result = make()
    else:
        assert marker
    return marker
