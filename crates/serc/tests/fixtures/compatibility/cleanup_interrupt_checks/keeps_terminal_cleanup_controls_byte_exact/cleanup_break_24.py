def f(manager, values, flag):
    try:
        work()
    except Error:
        try:
            while True:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
        return flag
    following()
    return flag
