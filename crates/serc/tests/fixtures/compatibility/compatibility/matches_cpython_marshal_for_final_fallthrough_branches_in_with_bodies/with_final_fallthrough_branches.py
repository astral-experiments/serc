def simple(manager, first, second):
    with manager:
        if first:
            value = 1
        elif second:
            value = 2
        else:
            value = 3
    return value


def boolean_elif(manager, first, second, third):
    with manager:
        if first:
            value = 1
        elif second and third:
            value = 2
        else:
            value = 3
    return value
