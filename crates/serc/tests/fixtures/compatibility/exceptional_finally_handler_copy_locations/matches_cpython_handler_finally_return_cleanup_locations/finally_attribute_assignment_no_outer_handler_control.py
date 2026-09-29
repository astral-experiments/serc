def f(load, manager, state, saved):
    try:
        with manager():
            return load()
    finally:
        state.value = saved
