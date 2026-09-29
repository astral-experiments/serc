def f(value, flag, other, items, manager):
    if (value == other if flag else other == flag):
        raise Error
    return other
