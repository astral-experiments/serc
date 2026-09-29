def f(value, enabled, manager):
    try:
        try:
            work()
        except Error as outer:
            try:
                retry()
            except Other:
                return value
    finally:
        cleanup()
