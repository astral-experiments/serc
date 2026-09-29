def f(value):
    if check(value) if not isinstance(value, (list, tuple)) else False:
        return None
    return str(value)
