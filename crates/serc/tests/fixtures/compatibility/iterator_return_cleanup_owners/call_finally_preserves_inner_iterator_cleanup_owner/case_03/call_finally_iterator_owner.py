def f(items, value):
    try:
        for item in items:
            try:
                return value
            except ValueError:
                recover()
    finally:
        cleanup()
