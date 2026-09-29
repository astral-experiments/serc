def f(manager, ready, test, work, after):
    with manager:
        try:
            if ready():
                while test():
                    work()
            after()
        except LookupError:
            raise
