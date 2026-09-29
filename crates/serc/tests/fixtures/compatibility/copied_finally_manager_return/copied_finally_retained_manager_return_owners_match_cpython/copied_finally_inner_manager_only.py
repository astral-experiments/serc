def exercise(outer, inner, value):
    try:
        with inner():
            if value is not None:
                work(value)
                return
            work(None)
        after()
    except Exception as error:
        if value is not None:
            record(error)
        raise
    finally:
        if value is not None:
            finish(value)
