def run(items, work, recover, stop, cleanup, keep):
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
                except ValueError:
                    pass
    return state
