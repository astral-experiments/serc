def f(flag):
    if flag:
        try:
            call(first=1)
        finally:
            call(final=2)
    else:
        call(last=3)
