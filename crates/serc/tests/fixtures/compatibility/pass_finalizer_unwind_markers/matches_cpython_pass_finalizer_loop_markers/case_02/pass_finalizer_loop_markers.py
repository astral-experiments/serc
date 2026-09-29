def collect(value, manager, items, flag):
    while flag:
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
