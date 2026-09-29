def f(value, flag, other, items, manager):
    try:
        work(value)
    finally:
        if (value == other if flag else other == flag):
            return None
    return other
