def f():
    try:
        try:
            work()
        except* ValueError as error:
            recover(error)
            pass
    except Exception as caught:
        finish(caught)
    after()
