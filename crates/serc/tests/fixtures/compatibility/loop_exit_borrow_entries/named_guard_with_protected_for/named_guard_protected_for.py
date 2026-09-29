def f(items, local):
    try:
        for item in items:
            result = work(item)
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
