def f(value, enabled):
    if enabled:
        try:
            work()
        except Error as error:
            if error.code:
                raise
        else:
            try:
                value = get_value()
                assert value == EXPECTED
            except BaseException:
                cancel()
                raise
            _, error = result()
            assert error == 0
    else:
        other()
    return value
