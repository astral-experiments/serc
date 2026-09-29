def f(manager, first, second, test, work, after):
    with manager:
        try:
            if first():
                if second():
                    while test():
                        work()
            after()
        except LookupError:
            raise
