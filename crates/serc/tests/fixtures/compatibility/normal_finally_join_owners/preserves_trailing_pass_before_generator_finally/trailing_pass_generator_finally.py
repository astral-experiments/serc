def f(value, close):
    try:
        yield value
        pass
    finally:
        close()
        close()
