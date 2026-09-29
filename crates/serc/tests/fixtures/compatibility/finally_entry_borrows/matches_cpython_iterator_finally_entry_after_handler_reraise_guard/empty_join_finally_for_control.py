def f(flag, values, consume):
    try:
        if flag:
            consume(1)
    except RuntimeError:
        pass
    finally:
        for value in values:
            consume(value)
