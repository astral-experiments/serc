def f(flag, values):
    for value in values:
        try:
            work()
        except Exception as error:
            if flag:
                report(error)
                break
        finish(value)
