def target(values, work):
    try:
        while True:
            try:
                value = next(values)
            except StopIteration:
                break
            work(value)
        work(None)
    except ValueError:
        work(False)
