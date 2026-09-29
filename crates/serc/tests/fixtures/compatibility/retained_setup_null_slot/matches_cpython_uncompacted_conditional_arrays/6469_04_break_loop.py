def f(state):
    while True:
        if state.cancel:
            break
    try:
        if state.future.done():
            state.future = work()
        yield state.future
    finally:
        state.count -= 1
