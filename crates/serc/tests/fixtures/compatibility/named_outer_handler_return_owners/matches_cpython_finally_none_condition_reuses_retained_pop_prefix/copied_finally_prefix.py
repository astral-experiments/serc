def copied_finally_prefix(value, first, second):
    try:
        consume(first, second)
        consume(first, second)
        consume(first, second)
        consume(first, second)
        return 0
    finally:
        if value is not None:
            finish(value)
