def f(flag, g):
    try:
        return 1e309-1e309
    finally:
        if flag:
            g()
