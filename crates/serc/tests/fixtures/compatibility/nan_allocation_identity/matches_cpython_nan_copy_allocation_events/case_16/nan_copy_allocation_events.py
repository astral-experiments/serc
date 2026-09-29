def f(flag):
    try:
        if flag:
            return 1
        return 2
    finally:
        def g(x=1e309-1e309):
            return x
