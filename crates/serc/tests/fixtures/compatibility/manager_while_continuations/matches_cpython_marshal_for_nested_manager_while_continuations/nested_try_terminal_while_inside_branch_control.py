def f(manager, ready, test, work, after):
    with manager:
        try:
            if ready():
                try:
                    while test():
                        work()
                except LookupError:
                    raise
            after()
        except TypeError:
            raise
