def f(value, flag, other, items, manager):
    try:
        work(value)
    finally:
        if ((value is None) if flag else (other is None)):
            return None
    return other
