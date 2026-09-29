def f():
    try:
        yield from ()
        return 2
    finally:
        raise RuntimeError('cleanup')
