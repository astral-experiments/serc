def target(values, work, done):
    while True:
        try:
            try:
                value = next(values)
            except StopIteration:
                break
            work(value)
        except ValueError:
            work(None)
            break
        if done():
            break
    for value in ():
        work(value)
