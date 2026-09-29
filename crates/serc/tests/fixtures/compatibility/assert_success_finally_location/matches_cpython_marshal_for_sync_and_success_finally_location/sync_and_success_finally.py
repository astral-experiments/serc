def f(left, right, close):
    try:
        assert left and right
    except Exception:
        raise
    finally:
        close()
