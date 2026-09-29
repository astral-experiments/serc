def f(flag, first, second):
    try:
        return 1
    finally:
        raise first if flag else second
