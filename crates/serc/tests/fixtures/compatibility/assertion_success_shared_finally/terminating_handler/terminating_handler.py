def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        raise
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
