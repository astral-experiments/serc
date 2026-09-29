def f(work, values, get):
    try:
        work()
    finally:
        for value in values:
            get(value)
