def f(flag, first, second, last):
    if flag:
        if first:
            work()
        elif second:
            recover()
        else:
            if not last():
                raise RuntimeError()
    else:
        other()
    after()
