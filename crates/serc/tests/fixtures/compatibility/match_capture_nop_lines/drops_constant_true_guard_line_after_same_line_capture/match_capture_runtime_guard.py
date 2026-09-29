def f(value):
    match value:
        case [item] if ready(item):
            pass
        case _:
            pass
