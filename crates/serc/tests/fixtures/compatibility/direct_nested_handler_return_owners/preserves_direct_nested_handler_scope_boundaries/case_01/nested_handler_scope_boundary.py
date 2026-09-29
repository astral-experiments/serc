def f():
    try:
        work()
    except Error:
        with manager:
            try:
                retry()
            except Other:
                return
