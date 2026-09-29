def f(flag, value):
    if flag:
        match value:
            case 'first':
                return first()
            case 'second':
                return second()
    else:
        other(value)
    return None
