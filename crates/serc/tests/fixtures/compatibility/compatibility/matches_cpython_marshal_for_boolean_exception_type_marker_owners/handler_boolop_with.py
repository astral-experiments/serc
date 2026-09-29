def f(manager):
    with manager:
        try:
            action()
        except A or B or C:
            recover()
