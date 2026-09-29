def f(manager, values, work, update):
    with manager:
        for value in values:
            try:
                work(value)
                break
            except ValueError:
                work(None)
            finally:
                state = update(value)
        else:
            raise RuntimeError
    return state
