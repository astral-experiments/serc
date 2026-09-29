def f(flag):
    try:
        yield 1
    except BaseException:
        raise
    finally:
        try:
            if flag:
                cleanup()
        except OSError:
            recover()
