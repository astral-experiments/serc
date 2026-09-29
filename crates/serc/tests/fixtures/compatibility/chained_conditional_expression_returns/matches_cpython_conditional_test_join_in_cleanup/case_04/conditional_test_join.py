def f(value, flag, other, items, manager):
    yield value
    if (check(value) if flag else False):
        return None
    yield other
