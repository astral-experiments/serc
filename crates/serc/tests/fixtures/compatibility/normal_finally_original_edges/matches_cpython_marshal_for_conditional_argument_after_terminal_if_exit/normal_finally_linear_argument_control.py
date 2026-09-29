def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        sink(error)
    return error
