def run(items, manager, work, recover, stop, cleanup, keep):
    with manager:
        state = None
        for item in items:
            try:
                state = work(item)
                break
            except LookupError as error:
                state = recover(error)
                if stop(error):
                    break
            finally:
                if state and not keep:
                    try:
                        cleanup(state)
                    except ValueError as cleanup_error:
                        if state is not None:
                            if state.error is None:
                                state.error = f"cleanup: {cleanup_error}"
                            else:
                                state.error = f"{state.error}; cleanup: {cleanup_error}"
    return state
