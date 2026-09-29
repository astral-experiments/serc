def f():
    try:
        work()
    except* ValueError as error:
        recover(error)
    else:
        pass
