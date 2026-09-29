def f():
    try:
        raise ExceptionGroup("group", [ValueError()])
    except* ValueError as error:
        recover(error)
    else:
        success()
