def direct_control(value):
    try:
        if value is not None:
            return 1
    except Exception:
        pass
    return 0


def compound(first, second, third, fourth, fifth):
    try:
        if ((first is not None and second is not None)
                or (third is not None and fourth is not None)
                or fifth is not None):
            return 1
    except Exception:
        pass
    return 0


def leading_comparisons(a, b, first, second):
    try:
        if a >= 0 and b >= 0 and (first is not None or second is not None):
            return 1
    except Exception:
        pass
    return 0
