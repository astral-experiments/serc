def f(flag,a,b):
    try:
        work()
        work()
        work()
        try:
            if flag:
                cleanup()
        finally:
            finish()
    finally:
        outer()
