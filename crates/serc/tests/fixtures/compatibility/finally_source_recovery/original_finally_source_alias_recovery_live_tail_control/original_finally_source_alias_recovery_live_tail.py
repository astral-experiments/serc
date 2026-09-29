def f(flag, primary, secondary, observe, cleanup):
    try:
        if flag:
            raise RuntimeError
        primary()
    except ValueError:
        try:
            secondary()
            observe()
        except TypeError:
            pass
        observe()
    finally:
        cleanup()
