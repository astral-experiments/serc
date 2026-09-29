def f(value, cleanup, cause):
    try:
        return value()
    finally:
        record()
        raise cleanup() from cause
