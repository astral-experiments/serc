def f():
    try:
        try:
            work()
        except* ValueError:
            recover()
            pass
    except Exception as caught:
        finish(caught)
    after()
