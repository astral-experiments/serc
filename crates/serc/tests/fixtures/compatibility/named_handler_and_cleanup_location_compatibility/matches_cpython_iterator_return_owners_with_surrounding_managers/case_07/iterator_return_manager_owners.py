def f(items, manager):
    with manager:
        try:
            for item in items:
                try:
                    work(item)
                except Error:
                    return item
        finally:
            finish()
