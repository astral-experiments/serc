def f(manager, value, flag):
    try:
        with manager:
            try:
                work()
            except ValueError:
                report(value)
                flag = True
                return
            if value:
                report(value)
                flag = True
                return
    except Exception as error:
        report(error)
        flag = True
    finally:
        if value is not None:
            finish()
