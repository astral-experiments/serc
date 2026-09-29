def f(work):
    try:
        return work()
    except AttributeError:
        error: str = "attribute"
    except ZeroDivisionError:
        error = "zero"
    work(error)
