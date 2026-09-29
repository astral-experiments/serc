def f(items, value):
    try:
        work()
    except Exception as error:
        for item in items:
            try:
                if item:
                    report(error)
                    return
            except LookupError:
                recover()
    finally:
        if value is not None:
            value.close()
