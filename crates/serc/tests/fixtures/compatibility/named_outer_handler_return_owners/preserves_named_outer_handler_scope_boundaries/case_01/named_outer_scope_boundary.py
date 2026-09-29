def f(value, enabled, manager):
    try:
        work()
    except Error:
        try:
            retry()
        except Other:
            return
