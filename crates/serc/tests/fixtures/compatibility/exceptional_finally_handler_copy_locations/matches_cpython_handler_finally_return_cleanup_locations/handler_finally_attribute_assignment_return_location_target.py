def f(load, manager, state, saved):
    try:
        return load()
    except ValueError:
        try:
            with manager():
                return load()
        finally:
            state.value = saved
