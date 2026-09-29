def f(items, manager):
    with manager:
        try:
            for item in items:
                try:
                    check(item)
                    return None
                except Error:
                    recover(item)
        finally:
            finish()
