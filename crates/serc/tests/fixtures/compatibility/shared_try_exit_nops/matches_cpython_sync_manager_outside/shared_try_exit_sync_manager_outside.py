def f(manager, flag):
    with manager:
        try:
            ping()
        except Exception:
            pong()
        finally:
            if flag:
                cleanup()
