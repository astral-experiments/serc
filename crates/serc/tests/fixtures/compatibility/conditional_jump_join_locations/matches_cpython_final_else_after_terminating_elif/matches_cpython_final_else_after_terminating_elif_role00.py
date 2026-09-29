def f(flag, first, second, last):
    if flag:
        if first:
            raise ValueError()
        elif second:
            raise TypeError()
        else:
            if not last():
                raise RuntimeError()
    else:
        other()
    after()
