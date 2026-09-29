def f(work, items):
    for item in items:
        try:
            work()
        except Exception as error:
            return error
        finally:
            return items
