def f(value, manager):
    with manager:
        try:
            work()
        except Error as error:
            if error.code:
                raise
        else:
            prepare()
            assert value == 0
        return value
