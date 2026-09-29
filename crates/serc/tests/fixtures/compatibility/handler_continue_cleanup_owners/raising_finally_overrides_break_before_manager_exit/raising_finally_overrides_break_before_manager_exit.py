def f(manager, values, stop, observe):
    for value in values:
        with manager():
            try:
                if stop(value):
                    break
                observe(value)
            finally:
                raise RuntimeError("stop")
