def f(value, enabled, manager):
    try:
        work()
    except Error as outer:
        try:
            retry()
        except Other as inner:
            return
