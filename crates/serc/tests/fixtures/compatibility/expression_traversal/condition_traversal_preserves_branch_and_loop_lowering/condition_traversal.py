def choose(value, flag, values):
    if not (not value):
        return 1 if flag else 2
    while value and flag:
        break
    return [item for item in values if not (not item) and flag]
