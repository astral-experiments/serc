def f(work, value):
    try:
        work()
        value = False
    except Exception as error:
        report(error)
    finally:
        return value
