def f(manager, values, flag):
    try:
        work()
    except Error:
        try:
            for item in values:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
        raise
    following()
    return flag
