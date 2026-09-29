def f(branch):
    try:
        first()
        second()
        third()
        fourth()
        fifth()
        value = SOURCE
        if branch:
            value = OTHER
        try:
            if ENABLED:
                consume(value)
        finally:
            inner_cleanup()
    finally:
        outer_cleanup()
