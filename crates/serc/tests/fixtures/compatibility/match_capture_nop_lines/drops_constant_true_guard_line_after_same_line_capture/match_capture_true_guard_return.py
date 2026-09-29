def f(value):
    match value:
        case Thing(item) if 1:
            return item
        case _:
            return None
