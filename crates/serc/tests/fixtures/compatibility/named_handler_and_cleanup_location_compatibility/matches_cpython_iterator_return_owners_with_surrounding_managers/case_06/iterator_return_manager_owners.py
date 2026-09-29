def f(items, manager):
    try:
        with manager:
            for item in items:
                return work(item)
    finally:
        finish()
