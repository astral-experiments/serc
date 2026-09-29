def sample(managers, on_error, cleanup):
    exited = False
    try:
        yield
    except BaseException as error:
        with managers():
            suppress = on_error(error)
        exited = True
        if not suppress:
            raise
    finally:
        if not exited:
            with managers():
                cleanup()
