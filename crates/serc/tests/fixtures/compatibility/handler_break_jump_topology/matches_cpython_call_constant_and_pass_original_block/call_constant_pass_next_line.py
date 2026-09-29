def f(enabled):
    try:
        if enabled:
            work()
            1; pass
    except Error:
        pass
    finish()
