def f():
    try:
        work()
    except* ValueError:
        recover()
    else:
        success()
    after()
