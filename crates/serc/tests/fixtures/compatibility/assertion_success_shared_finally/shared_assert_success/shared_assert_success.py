def f(item):
    try:
        work(item)
        assert left(item) or right(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
