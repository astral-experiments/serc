def f(flag):
    try:
        return 1
    finally:
        if flag:
            raise ValueError()
        else:
            raise RuntimeError()
