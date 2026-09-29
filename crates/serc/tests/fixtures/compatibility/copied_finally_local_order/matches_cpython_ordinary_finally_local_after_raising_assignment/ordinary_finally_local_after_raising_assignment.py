def f():
    try:
        prepare()
        value = create()
        work(value)
    except Exception:
        report()
        raise
    finally:
        before_cleanup()
        cleanup(value)
