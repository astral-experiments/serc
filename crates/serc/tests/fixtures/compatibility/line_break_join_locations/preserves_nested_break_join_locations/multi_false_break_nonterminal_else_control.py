def f(items, mode, first, second):
    for item in items:
        if mode:
            if first and second:
                break
        else:
            consume(mode)
        consume(item)
    return None
