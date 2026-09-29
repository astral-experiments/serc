def f(value, flag, other, task, manager):
    with manager:
        try:
            result = 1 if flag else 2
            work()
            work()
            work()
            if False:
                task(value)
        finally:
            if value:
                task(value)
    after()
