def probe(state, work, cleanup):
    try:
        if state == 0:
            if work():
                cleanup()
        elif state == 1:
            if work():
                cleanup()
        elif state == 2:
            try:
                if work():
                    cleanup()
                    work()
            except Exception:
                cleanup()
                raise
        elif state == 3:
            if work():
                cleanup()
        else:
            raise ValueError(state)
    except Exception:
        cleanup()
    return True
