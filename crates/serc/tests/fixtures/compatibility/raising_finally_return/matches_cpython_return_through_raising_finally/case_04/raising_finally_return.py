def f():
    try:
        yield 1
        x = 1
        return x
    finally:
        raise RuntimeError('cleanup')
