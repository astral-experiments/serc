def f(manager, lock, state, emit, mode):
    error = None
    with manager:
        with lock:
            if state.closed:
                raise Missing()
        state.updated = now()
        try:
            if mode == "read":
                result = emit(state)
            elif mode == "write":
                result = emit()
                if result:
                    with lock:
                        state.done = True
        except FirstError as exc:
            emit(str(exc))
            raise
        except Exception as exc:
            if getattr(exc, "special", False) is True:
                raise FirstError(str(exc)) from exc
            emit(str(exc))
            raise
        finally:
            if state.close_requested:
                try:
                    state.close()
                except Exception as exc:
                    error = str(exc) or type(exc).__name__
            state.updated = now()
    emit(result)
    if error is not None:
        emit(error)
    result.update(state.value)
    return result
