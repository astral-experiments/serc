def collect(value, manager, items, flag):
    for item in items:
        try:
            try:
                break
            finally:
                pass
        finally:
            with manager:
                pass
        after_iteration(value)
    after_loop(value)
