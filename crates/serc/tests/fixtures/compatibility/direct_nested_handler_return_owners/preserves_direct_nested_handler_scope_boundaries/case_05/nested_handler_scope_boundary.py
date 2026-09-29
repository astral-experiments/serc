def f():
    try:
        work()
    except Error:
        try:
            retry()
        except Other:
            try:
                return
            finally:
                cleanup()
