def f(work, items):
    for item in items:
        try:
            work()
        except Exception as error:
            return work
        finally:
            return items
