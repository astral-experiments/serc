def f(value):
    try:
        prepare()
        value = create()
        yield value
    except Exception:
        report()
        raise
    finally:
        before_cleanup()
        cleanup(value)
