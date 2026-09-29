def f():
    try:
        work()
    except Error as outer:
        try:
            retry()
        except Other as inner:
            return
