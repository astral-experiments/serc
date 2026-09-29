def f():
    try:
        first()
        second()
        third()
        fourth()
        fifth()
        value = SOURCE
        try:
            if ENABLED:
                consume(value)
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()
