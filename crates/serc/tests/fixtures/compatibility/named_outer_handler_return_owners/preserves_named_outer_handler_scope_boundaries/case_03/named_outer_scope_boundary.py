def f(value, enabled, manager):
    with manager:
        try:
            work()
        except Error as outer:
            try:
                retry()
            except Other:
                return value
