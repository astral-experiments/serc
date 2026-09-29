def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            return item
        else:
            return None
