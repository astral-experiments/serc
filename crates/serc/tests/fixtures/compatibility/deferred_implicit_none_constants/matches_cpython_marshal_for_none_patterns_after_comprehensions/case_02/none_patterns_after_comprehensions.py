def f(values):
    values = [x for x in values]
    for value in values:
        match value:
            case 1:
                call(value)
            case None:
                pass
    return values
