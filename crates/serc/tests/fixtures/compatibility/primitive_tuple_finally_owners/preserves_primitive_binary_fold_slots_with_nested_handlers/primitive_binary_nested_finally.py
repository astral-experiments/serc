def f(flag, manager):
    with manager:
        try:
            value = work('x' * 16)
            work()
        finally:
            try:
                if flag is not None:
                    cleanup(value)
            except Error:
                recover()
    return value
