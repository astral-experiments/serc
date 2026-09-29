def f(value, flag):
    try:
        work()
    except Error:
        recover()
    finally:
        if flag:
            consume(value)
