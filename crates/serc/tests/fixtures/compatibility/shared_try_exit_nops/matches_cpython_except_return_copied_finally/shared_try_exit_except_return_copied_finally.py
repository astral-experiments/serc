def f(flag):
    try:
        ping()
    except Exception:
        pong()
        return 1
    finally:
        if flag:
            cleanup()
