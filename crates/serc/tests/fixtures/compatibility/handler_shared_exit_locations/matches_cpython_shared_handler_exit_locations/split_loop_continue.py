def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag and value:
                report(error)
                continue
        if flag:
            finish(value)
