def terminating_body(manager, condition):
    with manager:
        try:
            raise Error()
        finally:
            if condition:
                cleanup()
