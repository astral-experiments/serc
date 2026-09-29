def f():
    try:
        action()
    except X:
        first()
    except A or B or C:
        recover()
