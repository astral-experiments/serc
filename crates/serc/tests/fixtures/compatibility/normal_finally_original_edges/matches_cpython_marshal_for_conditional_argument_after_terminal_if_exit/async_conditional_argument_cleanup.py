async def target(flag, work, sink):
    error = None
    try:
        if flag:
            return work()
    except Exception as caught:
        error = caught
    finally:
        sink("failure" if error else "success")
    return error
