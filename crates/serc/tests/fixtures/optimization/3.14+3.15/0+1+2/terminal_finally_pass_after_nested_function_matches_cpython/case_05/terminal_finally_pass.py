def f(flag):
    if False:
        missing = 1
    try:
        def inner():
            pass
        result = missing
        pass
    finally:
        cleanup()
