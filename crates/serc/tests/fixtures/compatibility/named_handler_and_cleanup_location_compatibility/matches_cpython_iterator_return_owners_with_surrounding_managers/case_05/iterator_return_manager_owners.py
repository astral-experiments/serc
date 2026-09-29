def f(items):
    try:
        for item in items:
            try:
                return work(item)
            except Error:
                recover(item)
    finally:
        finish()
