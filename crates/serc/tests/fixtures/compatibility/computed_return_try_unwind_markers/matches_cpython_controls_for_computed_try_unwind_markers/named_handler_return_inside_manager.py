def f(manager):
    with manager:
        try:
            work()
        except Error as error:
            return error
