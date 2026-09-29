def f(manager, values, flag):
    try:
        work()
    except Error as error:
        try:
            while True:
                before()
                if flag:
                    break
                after()
        except Other:
            ordinary()
    following()
    return flag
