import sys


def repair(state, key, pending, prepared, signaled, work, release, report):
    try:
        if not pending and not prepared and not signaled:
            with state.lock:
                report(key)
            return True
        work()
        return False
    finally:
        primary = sys.exc_info()[1]
        try:
            release()
        except Exception:
            if primary is None:
                raise
            report(primary)
        finally:
            with state.lock:
                if state.key == key:
                    state.key = None
