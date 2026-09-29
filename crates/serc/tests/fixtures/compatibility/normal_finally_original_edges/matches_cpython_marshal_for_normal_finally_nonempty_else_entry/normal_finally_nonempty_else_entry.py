def target(manager, cleanup, sink):
    result = None
    try:
        with manager:
            pass
    except Exception as error:
        result = error
    else:
        result = True
    finally:
        if cleanup is not None:
            cleanup.close()
        sink(result)


def control(manager, cleanup, sink):
    result = None
    try:
        with manager:
            pass
    except Exception as error:
        result = error
    else:
        result = True
    if cleanup is not None:
        cleanup.close()
    sink(result)
