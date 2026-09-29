def f(values, consume, should_raise):
    try:
        for value in values:
            consume(value)
    except RuntimeError:
        if should_raise():
            raise
    finally:
        for value in values:
            consume(value)
