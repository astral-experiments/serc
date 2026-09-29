def f(flag):
    try:
        if flag:
            work()
        pass
    except Error:
        recover()
    finally:
        finish()
    completed()
