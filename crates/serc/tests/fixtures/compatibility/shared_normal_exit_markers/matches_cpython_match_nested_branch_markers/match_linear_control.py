def f(subject, first, second, consume):
    match subject:
        case 0:
            value = first()
        case 1:
            value = second()
    consume(value)
    return value
