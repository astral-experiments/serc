def collect(value, manager, other, items, flag):
    for item in items:
        try:
            try:
                break
            finally:
                pass
        finally:
            with manager:
                consume(value)
        after_iteration(value)
    after_loop(value)
