def f(flag, items):
    try:
        work()
    except Exception:
        recover()
    else:
        for item in items:
            if item:
                break
        else:
            if flag:
                raise ValueError()
    after()
