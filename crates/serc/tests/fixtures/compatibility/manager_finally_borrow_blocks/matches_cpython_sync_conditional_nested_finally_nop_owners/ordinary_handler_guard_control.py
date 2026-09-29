def f(manager, enabled, work, close):
    with manager:
        try:
            work()
        finally:
            if enabled:
                try:
                    work()
                except ValueError:
                    close()
