def f():
    try:
        try:
            work()
        except Error:
            try:
                retry()
            except Other:
                return
    finally:
        cleanup()
