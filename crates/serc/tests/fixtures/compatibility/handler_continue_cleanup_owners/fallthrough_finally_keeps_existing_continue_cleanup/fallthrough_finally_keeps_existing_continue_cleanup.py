def f(manager, values, skip, observe, cleanup):
    for value in values:
        with manager():
            try:
                if skip(value):
                    continue
                observe(value)
            finally:
                cleanup(value)
