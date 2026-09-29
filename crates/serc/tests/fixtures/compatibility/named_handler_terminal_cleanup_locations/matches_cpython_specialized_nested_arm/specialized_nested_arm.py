def probe(action, state, emit):
    try:
        action()
    except Exception as error:
        if isinstance(error, ValueError):
            emit(error)
            if not state.done():
                state.finish(None)
        else:
            emit(error)
