def f(items, manager):
    with manager:
        try:
            for item in items:
                try:
                    return work(item)
                except Error:
                    recover(item)
        finally:
            finish()
