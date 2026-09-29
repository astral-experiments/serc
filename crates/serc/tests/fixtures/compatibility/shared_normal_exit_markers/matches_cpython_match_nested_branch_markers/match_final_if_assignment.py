def f(subject, flag, first, second, consume):
    match subject:
        case 0:
            value = first()
        case 1:
            if flag:
                value = first()
            else:
                value = second()
    consume(value)
    return value
