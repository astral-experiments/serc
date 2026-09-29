def f(manager, values, skip, observe):
    for value in values:
        with manager():
            try:
                if skip(value):
                    continue
                observe(value)
            finally:
                return "done"
