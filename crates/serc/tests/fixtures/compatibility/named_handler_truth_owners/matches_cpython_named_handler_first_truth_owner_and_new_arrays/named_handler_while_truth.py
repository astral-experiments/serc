def f(warn, flag, manager):
    try:
        work()
    except Error as e:
        while warn:
            work(e)
