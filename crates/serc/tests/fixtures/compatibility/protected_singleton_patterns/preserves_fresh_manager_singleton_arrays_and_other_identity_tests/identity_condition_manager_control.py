def f(manager, subject):
    with manager:
        if subject is False:
            return 0
        elif subject is True:
            return 1
        return 2
