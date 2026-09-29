def f(items, manager):
    for item in items:
        with manager:
            try:
                return work(item)
            finally:
                finish()
