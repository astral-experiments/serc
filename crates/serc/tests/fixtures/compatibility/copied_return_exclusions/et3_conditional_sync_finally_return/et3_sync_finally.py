def f(state):
    try:
        try:
            action()
        finally:
            if state is not None:
                state.clear()
    except Exception as err:
        report(err)
