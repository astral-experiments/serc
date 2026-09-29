def leading_terminating_body(manager, condition):
    with manager:
        before()
        try:
            if condition:
                raise Error()
            raise OtherError()
        finally:
            if condition:
                cleanup()
