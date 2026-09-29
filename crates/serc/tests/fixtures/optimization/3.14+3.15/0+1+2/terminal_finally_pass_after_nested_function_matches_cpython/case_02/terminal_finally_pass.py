def f(flag):
    try:
        def inner():
            return flag
        result = flag
        pass
    finally:
        cleanup(flag)
