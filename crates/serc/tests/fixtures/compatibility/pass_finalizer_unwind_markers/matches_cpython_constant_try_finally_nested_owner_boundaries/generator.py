def f():
    yield 0
    try:
        1
        2
    finally:
        close()
    yield 3
