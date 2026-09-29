def f(flag):
    try:
        if flag == 1:
            first()
            pass
        elif flag == 2:
            last()
            pass
    finally:
        cleanup()
    return flag
