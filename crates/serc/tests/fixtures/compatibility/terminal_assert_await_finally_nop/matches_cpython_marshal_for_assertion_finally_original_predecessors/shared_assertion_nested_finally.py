def f(left, right, cleanup, recover):
    try:
        assert left or right
    except Exception:
        raise
    finally:
        try:
            cleanup()
        except Exception:
            recover()
    return left
