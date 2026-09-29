def f(items, value):
    try:
        for item in items:
            return value
    except ValueError:
        recover()
    finally:
        cleanup()
