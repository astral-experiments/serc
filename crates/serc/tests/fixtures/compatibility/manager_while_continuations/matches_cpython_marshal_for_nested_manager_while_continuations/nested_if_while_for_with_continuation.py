def f(manager, values, ready, test, work, after):
    with manager:
        for value in values:
            if ready(value):
                while test():
                    work()
            after(value)
