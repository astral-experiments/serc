def collect(available, ready, acquire, load, release, finish):
    value = None
    try:
        value = ready()
        if not available():
            acquire()
            try:
                value = load()
            finally:
                if value:
                    try:
                        release()
                    except OSError:
                        pass
            finish(value)
    except RuntimeError:
        return None
    finish(value)
    return value
