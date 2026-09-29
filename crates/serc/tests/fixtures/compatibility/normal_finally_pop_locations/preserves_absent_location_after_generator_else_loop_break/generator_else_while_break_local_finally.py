def f(value, work, ready, cleanup):
    def finish():
        cleanup(value)
    try:
        work()
        yield value
    except BaseException:
        work()
        raise
    else:
        work()
        while True:
            work()
            if ready():
                work()
                break
            work()
    finally:
        finish()
