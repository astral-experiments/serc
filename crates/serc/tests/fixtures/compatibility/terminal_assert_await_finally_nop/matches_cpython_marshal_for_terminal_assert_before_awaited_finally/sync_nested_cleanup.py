def f(rows, cleanup, recover):
    failed = None
    try:
        assert rows == [0, 1, 2]
    except Exception as exc:
        failed = exc
        raise
    finally:
        try:
            cleanup()
        except Exception:
            if failed is None:
                raise
            recover()
    return rows
