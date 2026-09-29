def f(flag, value, ready):
    if flag:
        match value:
            case 'first':
                return first()
            case 'second' if ready:
                return second()
    else:
        other(value)
    return None
