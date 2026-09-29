def f(value):
    try:
        work()
    except Error as error:
        if error.code:
            raise
    else:
        prepare()
        assert value == 0
    return finish(value, other())
