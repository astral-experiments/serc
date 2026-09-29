def sample(managers, on_error, cleanup):
    exited = False
    try:
        yield
    finally:
        if not exited:
            with managers():
                cleanup()
