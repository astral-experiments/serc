def target(flag, choice, items, value, end):
    if flag:
        if choice:
            value = items[0]
        else:
            items[0] = value
    else:
        value = convert(value)
    return value, end
