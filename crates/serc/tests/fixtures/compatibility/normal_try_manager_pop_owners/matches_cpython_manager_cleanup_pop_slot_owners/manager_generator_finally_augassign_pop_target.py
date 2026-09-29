def f(manager, value, finish):
    with manager:
        try:
            yield value
        except ValueError:
            raise
        else:
            finish()
        finally:
            value -= 1
