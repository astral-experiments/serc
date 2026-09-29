def run(manager, other, flag, value, work):
    try:
        work(value)
    except Error:
        if flag:
            with manager:
                with other:
                    work(value)
            work(value)
        else:
            work(value)
    try:
        work(value)
    except Error:
        pass
    work(value)
    return value
