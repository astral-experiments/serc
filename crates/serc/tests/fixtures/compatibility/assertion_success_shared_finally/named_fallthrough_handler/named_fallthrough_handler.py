def f(item):
    try:
        work(item)
        assert not ready(item)
    except Error as error:
        recover(error)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
