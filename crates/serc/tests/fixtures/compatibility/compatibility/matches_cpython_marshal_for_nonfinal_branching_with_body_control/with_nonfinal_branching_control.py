def nonfinal(manager, first, second):
    with manager:
        if first:
            value = 1
        elif second:
            value = 2
        else:
            value = 3
        consume(value)
    return value
