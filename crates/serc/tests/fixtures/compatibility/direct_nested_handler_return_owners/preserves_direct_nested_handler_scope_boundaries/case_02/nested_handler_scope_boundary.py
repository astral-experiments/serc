def f():
    try:
        work()
    except Error:
        try:
            with manager:
                try:
                    retry()
                except Other:
                    return
        except Middle:
            recover()
