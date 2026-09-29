def f(value, flag, other, items, manager):
    for item in items:
        if (check(value) if flag else False):
            continue
        work(item)
    return other
