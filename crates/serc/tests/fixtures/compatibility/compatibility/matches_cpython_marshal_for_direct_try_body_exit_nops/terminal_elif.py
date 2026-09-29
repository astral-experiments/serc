def terminal_elif(first, second):
    try:
        if first <= 0:
            raise ValueError
        elif second <= 0:
            raise RuntimeError
    except TypeError:
        raise OSError from None
    return first
