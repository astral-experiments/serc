def f(flag, g):
    if flag:
        g()
    else:
        g()
    return ((1e309-1e309),)*2
