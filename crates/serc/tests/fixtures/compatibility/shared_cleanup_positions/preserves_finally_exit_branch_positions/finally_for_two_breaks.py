def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            try:
                get(value)
                break
            except LookupError:
                break
            except OSError:
                continue
