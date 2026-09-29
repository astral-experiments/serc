def f(manager, nested, ready, test, work, after):
    with manager:
        try:
            if ready():
                with nested:
                    while test():
                        work()
            after()
        except LookupError:
            raise
