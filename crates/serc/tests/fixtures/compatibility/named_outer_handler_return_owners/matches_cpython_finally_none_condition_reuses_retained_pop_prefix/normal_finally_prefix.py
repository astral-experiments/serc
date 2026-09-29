def normal_finally_prefix(value, first, second):
    try:
        consume(first, second)
        consume(first, second)
        consume(first, second)
        consume(first, second)
    finally:
        if value is not None:
            finish(value)
