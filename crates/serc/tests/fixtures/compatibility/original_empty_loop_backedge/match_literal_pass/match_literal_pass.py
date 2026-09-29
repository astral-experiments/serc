def f(items):
    for item in items:
        match item:
            case 1:
                pass
            case _:
                raise ValueError(item)
