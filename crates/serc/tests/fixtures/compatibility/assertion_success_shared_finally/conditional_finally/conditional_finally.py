def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
        if item is not None and ready(item):
            finish(item)
