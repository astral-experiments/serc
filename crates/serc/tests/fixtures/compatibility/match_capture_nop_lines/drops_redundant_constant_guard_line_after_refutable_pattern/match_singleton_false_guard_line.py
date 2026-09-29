def f(value):
    match value:
        case None if False:
            return 1
        case _:
            return 2
