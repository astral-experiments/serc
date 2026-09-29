def f(work, close, release):
    try:
        work()
    except Error:
        raise
    else:
        completed = True
    finally:
        try:
            close()
        finally:
            release()
