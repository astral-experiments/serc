def f(warn, flag, manager):
    try:
        work()
    except Error as e:
        if flag:
            work(e)
        if warn:
            other(e)
