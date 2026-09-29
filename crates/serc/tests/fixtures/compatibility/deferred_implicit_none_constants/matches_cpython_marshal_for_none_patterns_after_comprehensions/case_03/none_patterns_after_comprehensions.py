def f(values):
    for value in values:
        match value:
            case 1:
                call(value)
            case None:
                pass
        call(after=1)
    return values
