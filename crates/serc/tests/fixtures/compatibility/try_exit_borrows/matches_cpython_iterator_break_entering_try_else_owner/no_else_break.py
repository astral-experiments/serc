def no_else_break(values, stop):
    try:
        for value in values:
            if stop(value):
                break
    except Exception:
        report()
    after()
