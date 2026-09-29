def f(item):
    try:
        work(item)
        assert True or ready(item)
    except Error:
        recover(item)
    finally:
        if cleanup_ready(item):
            cleanup(item)
            wait(item)
