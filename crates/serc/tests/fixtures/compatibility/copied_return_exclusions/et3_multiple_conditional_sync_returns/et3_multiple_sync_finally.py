def f(state, ready):
    try:
        try:
            action()
        finally:
            if state is not None and ready and state.active:
                state.clear()
    except Exception as err:
        report(err)
