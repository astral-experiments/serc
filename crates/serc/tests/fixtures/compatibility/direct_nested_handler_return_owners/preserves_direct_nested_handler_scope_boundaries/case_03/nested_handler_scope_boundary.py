def f():
    try:
        work()
    except Error:
        try:
            retry()
        except Other as inner:
            return
