def f(flag):
    try:
        if flag:
            return 1
        return 2
    finally:
        def g():
            return 1e309-1e309
