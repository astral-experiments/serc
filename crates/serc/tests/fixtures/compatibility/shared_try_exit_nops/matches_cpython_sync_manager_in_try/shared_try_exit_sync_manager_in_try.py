def f(manager, flag):
    try:
        with manager:
            ping()
    except Exception:
        pong()
    finally:
        if flag:
            cleanup()
