def f(primary, secondary, observe, cleanup):
    try:
        primary()
    except ValueError:
        try:
            secondary()
            observe()
        except TypeError:
            pass
    finally:
        cleanup()
