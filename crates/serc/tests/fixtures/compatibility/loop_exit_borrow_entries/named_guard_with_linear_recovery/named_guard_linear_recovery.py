def f(value, local):
    try:
        result = work(value)
    except Error as error:
        if local:
            raise error
        report(error)
        result = []
    finally:
        if not local:
            sync()
            finish()
    return result
