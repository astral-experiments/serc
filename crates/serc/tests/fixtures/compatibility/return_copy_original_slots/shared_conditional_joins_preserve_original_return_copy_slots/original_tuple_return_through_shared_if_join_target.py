def target(flag, kind, choice, items, value, end):
    if flag:
        if kind == 1:
            if choice:
                value = items[0]
            else:
                items[0] = value
        elif kind == 2:
            if choice:
                value = items[1]
            else:
                items[1] = value
    else:
        value = convert(value)
        if choice:
            value = convert(value)
    return value, end
