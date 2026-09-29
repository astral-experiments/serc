def f(manager, values):
    for value in values:
        with manager:
            try:
                if first(value) and second(value) and value:
                    report(value)
                    return value
                else:
                    report(value)
            except Error:
                recover()
                continue
