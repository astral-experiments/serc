async def f(state):
    try:
        try:
            await action()
        finally:
            if state is not None:
                state.clear()
    except Exception as err:
        report(err)
