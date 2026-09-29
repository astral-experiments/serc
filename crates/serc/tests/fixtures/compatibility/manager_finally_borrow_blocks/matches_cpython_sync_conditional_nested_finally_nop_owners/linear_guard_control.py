def f(manager, enabled, work, close):
    with manager:
        try:
            work()
        finally:
            if enabled:
                close()
