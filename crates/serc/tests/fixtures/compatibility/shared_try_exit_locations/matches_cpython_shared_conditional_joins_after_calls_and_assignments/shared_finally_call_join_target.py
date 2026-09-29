def f(work, primary, fallback, build):
    try:
        work()
    finally:
        close = primary
        if callable(close):
            close()
        else:
            close = fallback
            if callable(close):
                close()
    return build(work, primary, fallback)
