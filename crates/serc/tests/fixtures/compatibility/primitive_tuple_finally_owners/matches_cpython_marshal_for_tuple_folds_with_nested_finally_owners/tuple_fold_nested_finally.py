def f(flag, other_flag, manager):
    try:
        try:
            x = work(('token_0', 'token_1'))
        finally:
            if flag is None:
                cleanup()
            else:
                other()
    finally:
        finish()
