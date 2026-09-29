def f(flag, value):
    if flag:
        match value:
            case 'first':
                first()
            case 'second':
                second()
        consume(value)
    else:
        other(value)
    return None
