def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag or value:
                report(error)
                continue
        finish(value)
