def case_0(item, value, bound, values, flag):
    if (
        0 <
        item < value
    ):
        return item
    else:
        return value

def case_10(item, value, bound, values, flag):
    if 0 < item < value < bound:
        return item
    else:
        return value

def case_20(item, value, bound, values, flag):
    if 0 < item is None:
        return item
    else:
        return value

def case_30(item, value, bound, values, flag):
    if (
        0 <
        item in [0, 1]
    ):
        return item
    else:
        return value

def case_40(item, value, bound, values, flag):
    if 0 < item in (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,):
        return item
    else:
        return value

def case_50(item, value, bound, values, flag):
    if 0 < item in (*values,):
        return item
    else:
        return value
