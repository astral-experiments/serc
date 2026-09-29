def f():
    try:
        yield 1
        pass
        return 2
    finally:
        raise RuntimeError('cleanup')
