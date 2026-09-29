def retained(flag):
    try:
        action()
    except Exception:
        failed()
    else:
        if flag:
            good()
        else:
            other()
    after()

def ordinary(flag):
    try:
        action()
    except Exception:
        failed()
    else:
        if flag:
            good()
    after()
