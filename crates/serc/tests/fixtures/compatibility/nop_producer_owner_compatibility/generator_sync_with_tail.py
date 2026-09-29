def f(manager):
    yield 0
    with manager:
        work()
        pass
    yield 1
