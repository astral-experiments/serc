def f(items):
    for item in items:
        match item:
            case True | False:
                pass
            case _:
                raise ValueError(item)
