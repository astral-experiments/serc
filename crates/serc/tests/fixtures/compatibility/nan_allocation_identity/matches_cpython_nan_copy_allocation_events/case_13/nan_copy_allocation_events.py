def f(flag, g):
    try:
        if flag:
            return 1
        return 2
    finally:
        g(((1e309-1e309),)*2)
