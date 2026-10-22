def recover(value):
    try:
        work()
    except Error:
        if 1:
            pass
        else:
            with manager:
                raise Failure
    return value
