def f(items):
    for item in items:
        match item:
            case True | False:
                work(item)
            case _:
                raise ValueError(item)
