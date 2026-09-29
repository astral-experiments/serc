def f(manager, value):
    with manager:
        try:
            consume(value)
            if value is not None:
                raise Error(value)
        except Error:
            recover()
            raise
    after()
