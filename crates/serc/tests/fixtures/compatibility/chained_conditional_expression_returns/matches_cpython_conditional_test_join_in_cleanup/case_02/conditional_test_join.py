def f(value, flag, other, items, manager):
    try:
        if (check(value) if flag else False):
            return None
        work(value)
    except Error:
        return other
    return value
