def f(value, cleanup, recover):
    try:
        assert value
    except Exception:
        recover()
    finally:
        try:
            cleanup()
        except Exception:
            recover()
    return value
