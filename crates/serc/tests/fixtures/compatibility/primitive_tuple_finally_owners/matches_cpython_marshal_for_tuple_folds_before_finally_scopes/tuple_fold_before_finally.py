def f(flag, other_flag, manager):
    x = work(('token_0',))
    try:
        work(flag)
    finally:
        if flag is None:
            cleanup()
        else:
            other()
