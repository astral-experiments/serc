def f(work, close):
    try:
        work()
        pass
    finally:
        close()
        close()
