def f(flag):
    try:
        def inner():
            pass
        result = flag; pass
    finally:
        cleanup()
