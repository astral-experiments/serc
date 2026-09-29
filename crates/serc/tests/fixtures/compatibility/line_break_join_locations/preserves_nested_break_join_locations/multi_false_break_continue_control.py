def f(items, mode, first, second):
    for item in items:
        if mode:
            if first and second:
                break
        else:
            continue
        consume(item)
    return None
