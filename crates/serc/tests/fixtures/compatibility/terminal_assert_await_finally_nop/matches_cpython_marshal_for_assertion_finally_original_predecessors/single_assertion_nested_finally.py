def f(value, cleanup, recover):
    try:
        assert value
    except Exception:
        raise
    finally:
        try:
            cleanup()
        except Exception:
            recover()
    return value
