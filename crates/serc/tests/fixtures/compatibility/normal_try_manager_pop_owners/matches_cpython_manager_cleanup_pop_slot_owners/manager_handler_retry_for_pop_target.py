def f(manager, inner, work, values, cleanup):
    with manager:
        try:
            with inner:
                work()
            return
        except ValueError:
            for value in values:
                try:
                    work(value)
                finally:
                    cleanup()
