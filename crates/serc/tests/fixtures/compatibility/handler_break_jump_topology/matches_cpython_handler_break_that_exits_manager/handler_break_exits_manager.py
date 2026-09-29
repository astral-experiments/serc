def f(manager, values):
    for value in values:
        with manager:
            try:
                consume(value)
            except LookupError:
                break
    finish()
