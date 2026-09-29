def probe(action, state, expected):
    try:
        action()
    except Exception as error:
        if not state.closed and state.generation == expected:
            state.error = error
