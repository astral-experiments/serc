def f(items, action, consume):
    for value in items:
        try:
            if not action(value):
                continue
        finally:
            if value:
                try:
                    consume(value)
                except Exception:
                    pass
