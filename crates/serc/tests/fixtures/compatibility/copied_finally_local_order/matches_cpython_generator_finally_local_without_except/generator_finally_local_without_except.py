def f():
    try:
        prepare()
        value = create()
        yield value
    finally:
        before_cleanup()
        cleanup(value)
