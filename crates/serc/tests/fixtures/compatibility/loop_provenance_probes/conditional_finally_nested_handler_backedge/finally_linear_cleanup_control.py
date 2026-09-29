def f(items, action, consume):
    for value in items:
        try:
            action(value)
        finally:
            if value:
                consume(value)
