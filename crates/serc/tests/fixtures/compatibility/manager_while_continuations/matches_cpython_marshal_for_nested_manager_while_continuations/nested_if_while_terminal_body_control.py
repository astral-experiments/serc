def f(manager, ready, test, work):
    with manager:
        if ready():
            while test():
                work()
