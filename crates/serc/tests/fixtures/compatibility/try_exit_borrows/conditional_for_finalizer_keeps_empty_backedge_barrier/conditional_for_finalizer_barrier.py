def f(manager, items, value, flag, work, observe):
    with manager:
        try:
            work(value)
        finally:
            for item in items:
                if flag:
                    item.cancel()
    observe(value)
    observe(value)
    return value
