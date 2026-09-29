def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        (sink if error else work)(error)
    return error
