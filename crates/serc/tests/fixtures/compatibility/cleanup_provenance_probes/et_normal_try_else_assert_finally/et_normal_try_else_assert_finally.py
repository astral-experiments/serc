def f(manager, value):
    try:
        try:
            value = read(value)
            assert value.ready == 1
        except Error as error:
            if recover(error):
                raise
            return fallback()
        else:
            with manager:
                reset()
            return value
    finally:
        if value:
            value.close()
