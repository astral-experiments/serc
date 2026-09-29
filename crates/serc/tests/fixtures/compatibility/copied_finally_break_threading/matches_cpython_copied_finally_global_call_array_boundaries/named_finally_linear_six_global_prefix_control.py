def run(items, manager, recover, stop, cleanup, keep, clock,
        flag, work):
    with manager:
        state = None
        for item in items:
            try:
                work(item)
                state = Record(
                    first=str(fetch(item)),
                    second=str(classify(item)),
                    elapsed=round(clock.now(), 3),
                )
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
