def f(work):
    try:
        return work()
    except AttributeError:
        error: str
    except ZeroDivisionError:
        error = "failed"
    work(error)
