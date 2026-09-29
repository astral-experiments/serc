def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            try:
                get(value)
            except LookupError:
                break
        get(None)
